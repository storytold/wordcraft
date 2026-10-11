//! Typing behaviour checked side by side against Word: keystrokes go through the same commands
//! as the keyboard (one `text.insert` per character, Enter, Tab, Backspace), and the expected
//! results are what Word produces for the same keys.

use serde_json::json;
use wordcraft_doc::props::{BorderStyle, VertAlign};
use wordcraft_doc::{Pos, StoryRef};

use crate::Session;

fn s() -> Session {
    Session::new(wordcraft_doc::Document::new())
}

fn run(s: &mut Session, id: &str, v: serde_json::Value) {
    s.run(id, &v).unwrap_or_else(|e| panic!("{id}: {e}"));
}

/// Type like a person: `\n` is Enter, `\t` is Tab, `\u{8}` is Backspace, everything else one
/// character at a time.
fn typ(s: &mut Session, keys: &str) {
    for c in keys.chars() {
        match c {
            '\n' => run(s, "text.newParagraph", json!({})),
            '\t' => run(s, "text.tab", json!({})),
            '\u{8}' => run(s, "text.backspace", json!({})),
            c => run(s, "text.insert", json!({"text": c.to_string()})),
        }
    }
}

/// Body paragraphs as (text, list level if in a list, in "List Paragraph").
fn paras(s: &Session) -> Vec<(String, Option<u8>, bool)> {
    s.doc
        .body
        .iter()
        .filter_map(|b| b.as_para())
        .map(|p| (p.text.clone(), p.props.numbering.filter(|n| n.num != 0).map(|n| n.level), p.props.style.as_deref() == Some("ListParagraph")))
        .collect()
}

fn para(text: &str, level: Option<u8>) -> (String, Option<u8>, bool) {
    (text.to_string(), level, level.is_some())
}

fn typed(keys: &str) -> Session {
    let mut s = s();
    typ(&mut s, keys);
    s
}

#[test]
fn enter_twice_after_a_list_item_ends_the_list() {
    let s = typed("* item\n\ntext");
    assert_eq!(paras(&s), vec![para("Item", Some(0)), para("text", None)]);
}

#[test]
fn more_enters_after_a_list_make_blank_paragraphs() {
    // The "stuck Enter" bug: after the list ended, further Enters did nothing.
    let s = typed("* item\n\n\ntext");
    assert_eq!(paras(&s), vec![para("Item", Some(0)), para("", None), para("text", None)]);
    let s = typed("1. item\n\n\n\nafter");
    assert_eq!(paras(&s), vec![para("Item", Some(0)), para("", None), para("", None), para("after", None)]);
}

#[test]
fn the_paragraph_after_a_list_is_normal_text() {
    let mut s = typed("* Apple\nBanana\n\nAfter list");
    let p = s.doc.para_at(&s.sel.focus).unwrap();
    assert_eq!(p.props.style, None, "back to Normal, not an indented List Paragraph");
    assert_eq!(p.props.numbering, None);
    // And the next paragraph too.
    typ(&mut s, "\nMore");
    assert_eq!(paras(&s).last(), Some(&para("More", None)));
}

#[test]
fn enter_on_an_empty_nested_item_moves_it_up_a_level_first() {
    let s = typed("* a\n\tb\n\ntext");
    assert_eq!(paras(&s), vec![para("A", Some(0)), para("B", Some(1)), para("text", Some(0))]);
    let s = typed("* a\n\tb\n\n\ntext");
    assert_eq!(paras(&s), vec![para("A", Some(0)), para("B", Some(1)), para("text", None)]);
    let s = typed("* a\n\tb\n\n\n\ntext");
    assert_eq!(paras(&s), vec![para("A", Some(0)), para("B", Some(1)), para("", None), para("text", None)]);
    let s = typed("* a\n\tb\n\tc\n\n\n\ntext");
    assert_eq!(paras(&s), vec![para("A", Some(0)), para("B", Some(1)), para("C", Some(2)), para("text", None)]);
    let s = typed("1. a\n\tb\n\n\ntext");
    assert_eq!(paras(&s), vec![para("A", Some(0)), para("B", Some(1)), para("text", None)]);
}

#[test]
fn shift_tab_and_tab_change_list_levels() {
    let mut s = typed("1. One\nTwo\n\tSub a\nSub b\n");
    run(&mut s, "text.backTab", json!({}));
    typ(&mut s, "Three\n\nDone");
    assert_eq!(
        paras(&s),
        vec![para("One", Some(0)), para("Two", Some(0)), para("Sub a", Some(1)), para("Sub b", Some(1)), para("Three", Some(0)), para("Done", None)]
    );
}

#[test]
fn backspace_at_the_start_of_an_item_removes_the_number_then_the_indent() {
    let mut s = typed("* One\nTwo");
    run(&mut s, "caret.home", json!({}));
    typ(&mut s, "\u{8}");
    // Number gone, still indented with the list.
    assert_eq!(paras(&s)[1], ("Two".to_string(), None, true));
    typ(&mut s, "\u{8}|");
    // Second Backspace: Normal paragraph, not joined to the one above.
    assert_eq!(paras(&s), vec![para("One", Some(0)), para("|Two", None)]);
}

#[test]
fn backspace_on_an_empty_item_keeps_the_paragraph() {
    let s = typed("* One\n\u{8}Z");
    assert_eq!(paras(&s), vec![para("One", Some(0)), ("Z".to_string(), None, true)]);
}

#[test]
fn enter_in_the_middle_or_at_the_start_of_an_item() {
    let mut s = typed("* Hello world");
    for _ in 0..5 {
        run(&mut s, "caret.left", json!({}));
    }
    typ(&mut s, "\nX");
    assert_eq!(paras(&s), vec![para("Hello ", Some(0)), para("Xworld", Some(0))]);
    let mut s = typed("* first\nsecond");
    run(&mut s, "caret.home", json!({}));
    typ(&mut s, "\nY");
    assert_eq!(paras(&s), vec![para("First", Some(0)), para("", Some(0)), para("Ysecond", Some(0))]);
}

fn label_at(s: &Session, i: usize) -> String {
    let n = s.doc.para_at(&Pos::body(i, 0)).and_then(|p| p.props.numbering).unwrap();
    s.doc.numbering.level(n.num, n.level).map(|l| l.text.clone()).unwrap_or_default()
}

#[test]
fn list_autoformat_triggers() {
    let s = typed("- Item\nNext");
    assert_eq!(label_at(&s, 0), "-");
    assert_eq!(paras(&s), vec![para("Item", Some(0)), para("Next", Some(0))]);
    let s = typed("1) a\nb");
    assert_eq!(label_at(&s, 0), "%1)");
    let s = typed("a. alpha\nbeta");
    assert_eq!(paras(&s), vec![para("Alpha", Some(0)), para("beta", Some(0))]);
    assert_eq!(label_at(&s, 0), "%1.");
    let n = s.doc.para_at(&Pos::body(0, 0)).and_then(|p| p.props.numbering).unwrap();
    assert_eq!(s.doc.numbering.level(n.num, 0).map(|l| l.format), Some(wordcraft_doc::section::NumFormat::LowerLetter));
}

#[test]
fn typing_1_dot_starts_numbering_again() {
    let s = typed("1. a\nb\n\nmid\n1. c\nd");
    let num = |i: usize| s.doc.para_at(&Pos::body(i, 0)).and_then(|p| p.props.numbering).map(|n| n.num);
    assert_eq!(num(0), num(1));
    assert_eq!(num(3), num(4));
    assert_ne!(num(0), num(3), "a new list restarts at 1");
}

#[test]
fn undo_right_after_autoformat_undoes_only_the_autoformat() {
    let mut s = typed("* ");
    assert!(paras(&s)[0].1.is_some());
    run(&mut s, "edit.undo", json!({}));
    typ(&mut s, "x");
    assert_eq!(paras(&s), vec![("* x".to_string(), None, false)]);
    // Same for AutoCorrect.
    let mut s = typed("teh ");
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(s.doc.plain_text(StoryRef::Body), "teh ");
}

#[test]
fn autocorrect_as_you_type() {
    let s = typed("teh cat said hello. world (c) \"quoted\" don't -- yes wait... i think so. e.g. this. ");
    assert_eq!(s.doc.plain_text(StoryRef::Body), "The cat said hello. World © “quoted” don’t – yes wait… I think so. E.g. this. ");
    // Abbreviations and initials don't end sentences.
    let s = typed("see e.g. the map. Ask J. smith or Dr. jones ");
    assert_eq!(s.doc.plain_text(StoryRef::Body), "See e.g. the map. Ask J. smith or Dr. jones ");
    // Fractions; "iPhone" and file names stay as typed.
    let s = typed("1/2 cup. iPhone. see readme.txt ");
    assert_eq!(s.doc.plain_text(StoryRef::Body), "½ cup. iPhone. See readme.txt ");
}

#[test]
fn enter_finishes_a_word_for_autocorrect() {
    let s = typed("hello\nteh end\n");
    assert_eq!(s.doc.plain_text(StoryRef::Body), "Hello\nThe end\n");
}

#[test]
fn ordinals_become_superscript() {
    let s = typed("the 1st and 22nd and 13th but not 1th ");
    let p = s.doc.para_at(&Pos::body(0, 0)).unwrap();
    let sup = |t: &str| {
        let i = p.text.find(t).unwrap() + t.len() - 2;
        p.props_of_char(i).vert_align == Some(VertAlign::Superscript)
    };
    assert!(sup("1st") && sup("22nd") && sup("13th"));
    assert!(!sup("1th"));
}

#[test]
fn web_addresses_become_links() {
    let mut s = typed("see https://example.org/log, then ");
    let p = s.doc.para_at(&Pos::body(0, 0)).unwrap();
    let at = p.text.find("https").unwrap();
    assert_eq!(p.props_of_char(at).link.as_deref(), Some("https://example.org/log"));
    assert_eq!(p.props_of_char(p.text.find(',').unwrap()).link, None, "trailing comma stays outside");
    typ(&mut s, "more");
    let p = s.doc.para_at(&Pos::body(0, 0)).unwrap();
    assert_eq!(p.props_of_char(p.text.find("more").unwrap()).link, None);
}

#[test]
fn dashes() {
    let s = typed("a -- b and c - d and e--f ");
    assert_eq!(s.doc.plain_text(StoryRef::Body), "A – b and c – d and e—f ");
}

#[test]
fn border_line_autoformat() {
    let s = typed("Above\n---\nBelow");
    assert_eq!(s.doc.plain_text(StoryRef::Body), "Above\nBelow");
    let b = s.doc.para_at(&Pos::body(0, 0)).and_then(|p| p.props.borders).and_then(|b| b.bottom).unwrap();
    assert_eq!(b.style, BorderStyle::Single);
    assert!(s.doc.para_at(&Pos::body(1, 0)).unwrap().props.borders.is_none());
    for (keys, style) in [("===", BorderStyle::Double), ("***", BorderStyle::Dotted), ("___", BorderStyle::Thick)] {
        let s = typed(&format!("x\n{keys}\n"));
        let b = s.doc.para_at(&Pos::body(0, 0)).and_then(|p| p.props.borders).and_then(|b| b.bottom);
        assert_eq!(b.map(|b| b.style), Some(style), "{keys}");
    }
    // At the top of the document the line goes under that paragraph and typing continues below.
    let s = typed("---\nText");
    assert_eq!(s.doc.plain_text(StoryRef::Body), "\nText");
    assert!(s.doc.para_at(&Pos::body(0, 0)).and_then(|p| p.props.borders).is_some());
    assert!(s.doc.para_at(&Pos::body(1, 0)).unwrap().props.borders.is_none());
}

#[test]
fn a_large_mixed_document_types_like_word() {
    // The corpus typed into Word and WordCraft side by side; the paragraph structure must match.
    let mut s = s();
    let prose = "The keeper's log runs to four hundred pages, and almost every one begins with the weather. ";
    typ(&mut s, &format!("Notes\n{}\n", prose.repeat(20)));
    typ(&mut s, "The supplies were:\n* Lamp oil, 1/2 barrel\nWicks of the 1st quality\n\tSpare chimneys\nSpare springs\n");
    run(&mut s, "text.backTab", json!({}));
    typ(&mut s, "Salt beef\n\nThe schedule:\n1. Signal the boat\nLower the stores\n\tCheck the net\n");
    run(&mut s, "text.backTab", json!({}));
    typ(&mut s, "Sign the manifest\n\n---\nIn March the storm broke -- and he slept.\n");
    typ(&mut s, "a. Inspect the glass\nReplace the seals\n\n");
    typ(&mut s, &format!("Afterword. teh keeper retired. {}\n* one more\nsecond\n\n\nFinal.", prose.repeat(10)));
    let got: Vec<(String, Option<u8>)> = paras(&s).into_iter().map(|(t, l, _)| (t.chars().take(24).collect(), l)).collect();
    let want: Vec<(&str, Option<u8>)> = vec![
        ("Notes", None),
        ("The keeper’s log runs to", None),
        ("The supplies were:", None),
        ("Lamp oil, ½ barrel", Some(0)),
        ("Wicks of the 1st quality", Some(0)),
        ("Spare chimneys", Some(1)),
        ("Spare springs", Some(1)),
        ("Salt beef", Some(0)),
        ("The schedule:", None),
        ("Signal the boat", Some(0)),
        ("Lower the stores", Some(0)),
        ("Check the net", Some(1)),
        ("Sign the manifest", Some(0)),
        ("In March the storm broke", None),
        ("Inspect the glass", Some(0)),
        ("Replace the seals", Some(0)),
        ("Afterword. The keeper re", None),
        ("One more", Some(0)),
        ("Second", Some(0)),
        ("", None),
        ("Final.", None),
    ];
    let want: Vec<(String, Option<u8>)> = want.into_iter().map(|(t, l)| (t.to_string(), l)).collect();
    assert_eq!(got, want);
    // Every non-list paragraph is Normal (no list indent left behind).
    for (t, l, lp) in paras(&s) {
        assert!(l.is_some() || !lp, "{t:?} still a List Paragraph");
    }
}

#[test]
fn enter_right_after_starting_a_list_cancels_it() {
    let s = typed("* \nPlain");
    assert_eq!(paras(&s), vec![para("Plain", None)]);
}

#[test]
fn shift_enter_stays_inside_the_list_item() {
    let mut s = typed("* line one");
    run(&mut s, "text.lineBreak", json!({}));
    typ(&mut s, "cont\ntwo");
    assert_eq!(paras(&s), vec![para("Line one\ncont", Some(0)), para("two", Some(0))]);
}

/// Body paragraph texts, lower-cased (AutoCorrect may capitalise a sentence start).
fn texts(s: &Session) -> Vec<String> {
    paras(s).into_iter().map(|p| p.0.to_lowercase()).collect()
}

#[test]
fn tracked_enter_and_backspace_track_the_paragraph_mark() {
    // Issue #229: with Track Changes on, Enter inserts a tracked paragraph mark that Reject All
    // takes out again; Backspace over your own Enter removes it outright.
    let mut s = typed("one");
    run(&mut s, "review.trackChanges", json!({"value": true}));
    typ(&mut s, "\ntwo");
    assert_eq!(texts(&s), ["one", "two"]);
    typ(&mut s, "\u{8}\u{8}\u{8}\u{8}");
    assert_eq!(texts(&s), ["one"]);
    typ(&mut s, "\n");
    assert_eq!(texts(&s), ["one", ""]);
    run(&mut s, "review.rejectAll", json!({}));
    assert_eq!(texts(&s), ["one"]);
    // A break that was already there isn't removed: Backspace marks it deleted and moves before it.
    let mut s = typed("one\ntwo");
    run(&mut s, "review.trackChanges", json!({"value": true}));
    run(&mut s, "caret.set", json!({"pos": {"block": 1, "off": 0}}));
    typ(&mut s, "\u{8}");
    assert_eq!(texts(&s), ["one", "two"]);
    assert_eq!(s.sel.focus, Pos::body(0, 3));
    assert!(s.doc.para_at(&Pos::body(0, 0)).is_some_and(|p| p.mark.del.is_some()));
}

#[test]
fn reject_all_takes_out_the_paragraph_breaks_a_tracked_paste_inserted() {
    // Issue #417: Paste's paragraph marks weren't tracked, so Reject All left the split.
    for text in ["X\nY", "\n"] {
        let mut s = s();
        run(&mut s, "text.insert", json!({"text": "AB", "raw": true}));
        run(&mut s, "caret.set", json!({"pos": {"block": 0, "off": 1}}));
        run(&mut s, "review.trackChanges", json!({"value": true}));
        run(&mut s, "edit.paste", json!({"text": text}));
        let (head, tail) = text.split_once('\n').unwrap_or_default();
        assert_eq!(texts(&s), [format!("a{}", head.to_lowercase()), format!("{}b", tail.to_lowercase())], "{text:?}");
        let pasted = texts(&s);
        run(&mut s, "review.rejectAll", json!({}));
        assert_eq!(texts(&s), ["ab"], "{text:?}");
        assert!(s.doc.revisions.is_empty(), "{text:?}");
        run(&mut s, "edit.undo", json!({}));
        run(&mut s, "review.acceptAll", json!({}));
        assert_eq!(texts(&s), pasted, "{text:?}: accepting keeps the break");
    }
}

#[test]
fn plain_text_paste_keeps_an_empty_paragraph_in_a_list() {
    // Issue #422: the pasted paragraph separators went through Enter, which ends the list on an
    // empty item, so `A\n\nB` lost its empty paragraph.
    for (id, v) in [("edit.pasteText", json!({"text": "A\n\nB"})), ("edit.pasteSpecial", json!({"as": "text", "text": "A\n\nB"}))] {
        let mut s = s();
        run(&mut s, "para.bullets", json!({}));
        run(&mut s, id, v);
        assert_eq!(texts(&s), ["a", "", "b"], "{id}");
    }
    // Enter twice still ends the list.
    let mut s = s();
    run(&mut s, "para.bullets", json!({}));
    typ(&mut s, "A\n\nB");
    assert_eq!(paras(&s), vec![para("A", Some(0)), para("B", None)]);
}
