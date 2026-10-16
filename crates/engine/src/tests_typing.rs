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
fn sample_text_formulas_expand_on_enter() {
    for name in ["rand", "lorem"] {
        for (args, paragraphs, sentences) in [
            ("", 5, 3),
            ("2", 2, 3),
            (" 2 ", 2, 3),
            ("2,4", 2, 4),
            ("2, 4", 2, 4),
            (" 2 , 4 ", 2, 4),
            ("1,1", 1, 1),
            ("200,1", 200, 1),
            ("1,200", 1, 200),
        ] {
            let mut s = typed(&format!("={name}({args})"));
            let depth = s.undo_depth();
            typ(&mut s, "\n");
            let generated: Vec<_> = paras(&s).into_iter().map(|p| p.0).collect();
            assert_eq!(generated.len(), paragraphs + 1, "{name}({args})");
            for text in generated.iter().take(paragraphs) {
                assert_eq!(text.matches('.').count(), sentences);
                assert!(text.starts_with(if name == "lorem" { "Lorem ipsum" } else { "A blank page" }), "{name}({args}): {text:?}");
            }
            assert_eq!(generated.last().unwrap(), "");
            assert_eq!(s.sel.focus, Pos::body(paragraphs, 0));
            assert_eq!(s.undo_depth(), depth + 1);
            let expanded = s.doc.clone();
            run(&mut s, "edit.undo", json!({}));
            assert_eq!(texts(&s), [format!("={name}({args})")]);
            run(&mut s, "edit.redo", json!({}));
            assert_eq!(s.doc, expanded);
        }
    }
}

#[test]
fn sample_text_invalid_parameters_use_defaults() {
    for name in ["rand", "lorem"] {
        for args in ["0", "-1", "201", "abc", "1.5", "99999999999999999999999999999", "2,0", "0,4", "2,201", "2,no", "2,", ",4", "1,2,3"] {
            let s = typed(&format!("={name}({args})\n"));
            let texts = texts(&s);
            assert_eq!(texts.len(), 6, "{name}({args})");
            assert!(texts.iter().take(5).all(|text| text.matches('.').count() == 3));
        }
    }
}

#[test]
fn sample_text_requires_an_exact_paragraph_and_caret_at_end() {
    for formula in [" =rand()", "=rand() ", "prefix =rand()", "=lorem() suffix", "=other()", "=rand(", "=rand(())"] {
        assert_eq!(texts(&typed(&format!("{formula}\n"))), [formula, ""]);
    }
    let mut s = typed("=rand()");
    run(&mut s, "caret.home", json!({}));
    typ(&mut s, "\n");
    assert_eq!(texts(&s), ["", "=rand()"]);
    let mut s = typed("=rand()suffix");
    s.sel = crate::Selection { anchor: Pos::body(0, 7), focus: Pos::body(0, 13) };
    typ(&mut s, "\n");
    assert_eq!(texts(&s), ["=rand()", ""]);
}

#[test]
fn sample_text_preserves_typing_and_paragraph_formatting() {
    let mut s = s();
    run(&mut s, "format.bold", json!({}));
    run(&mut s, "para.alignCenter", json!({}));
    typ(&mut s, "=rand(2,1)\n");
    for p in s.doc.body.iter().filter_map(|b| b.as_para()).take(2) {
        assert_eq!(p.props.align, Some(wordcraft_doc::props::Align::Center));
        assert_eq!(p.props_at(0).bold, Some(true));
    }
}

#[test]
fn sample_text_replacement_tracks_changes() {
    let mut s = typed("=lorem(2,1)");
    run(&mut s, "review.trackChanges", json!({}));
    typ(&mut s, "\n");
    assert!(s.doc.body.iter().filter_map(|b| b.as_para()).flat_map(|p| &p.runs).any(|r| r.props.ins.is_some()));
    run(&mut s, "review.rejectAll", json!({}));
    assert_eq!(texts(&s), ["=lorem(2,1)"]);
    run(&mut s, "edit.undo", json!({}));
    run(&mut s, "review.acceptAll", json!({}));
    assert_eq!(texts(&s).len(), 3);
    assert!(!s.doc.plain_text(StoryRef::Body).contains("=lorem"));
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
    let text_x = |s: &mut Session| s.layout().caret(&Pos::body(1, 0)).map(|c| c.x);
    let before = text_x(&mut s);
    typ(&mut s, "\u{8}");
    // Number gone, still indented with the list: the text stays where it was.
    assert_eq!(paras(&s)[1], ("Two".to_string(), None, true));
    assert_eq!(text_x(&mut s), before);
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

/// #835: the English replacement list applies only to English text. Polish "i" ("and") stays
/// lowercase; symbols still change; English and untagged text still get "I".
#[test]
fn autocorrect_replacement_list_follows_the_texts_language() {
    // Polish as the document's default language.
    let mut s = s();
    run(&mut s, "review.language", json!({"lang": "pl-PL", "default": true}));
    typ(&mut s, "Ala i kot. kawa i herbata teh (c) ");
    assert_eq!(s.doc.plain_text(StoryRef::Body), "Ala i kot. Kawa i herbata teh © ");
    // Polish on the run only: the word's own language counts.
    let mut s = self::s();
    typ(&mut s, "Ala");
    run(&mut s, "select.all", json!({}));
    run(&mut s, "review.language", json!({"lang": "pl-PL"}));
    run(&mut s, "caret.docEnd", json!({}));
    typ(&mut s, " i kot ");
    assert_eq!(s.doc.plain_text(StoryRef::Body), "Ala i kot ");
    // English (any region) and untagged text (the default proofing language is English).
    let mut s = self::s();
    run(&mut s, "review.language", json!({"lang": "en-GB", "default": true}));
    typ(&mut s, "so i think teh end ");
    assert_eq!(s.doc.plain_text(StoryRef::Body), "So I think the end ");
    let s = typed("so i think teh end ");
    assert_eq!(s.doc.plain_text(StoryRef::Body), "So I think the end ");
    // A Polish interface: a new document is Polish by its default language, and text with no
    // language at all (a .docx without `w:lang`) follows the editing language too.
    let mut s = self::s();
    s.set_editing_language(Some("pl-PL"));
    typ(&mut s, "Ala i kot ");
    assert_eq!(s.doc.plain_text(StoryRef::Body), "Ala i kot ", "document default from the editing language");
    let mut s = self::s();
    s.set_editing_language(Some("pl-PL"));
    s.doc.styles.default_chr.lang = None;
    typ(&mut s, "Ala i kot ");
    assert_eq!(s.doc.plain_text(StoryRef::Body), "Ala i kot ", "untagged text in the editing language");
    // `eng` is English too, as for proofing.
    let mut s = self::s();
    run(&mut s, "review.language", json!({"lang": "eng", "default": true}));
    typ(&mut s, "so i think ");
    assert_eq!(s.doc.plain_text(StoryRef::Body), "So I think ");
}

#[test]
fn autocorrect_options_change_what_typing_does() {
    let mut s = s();
    // A user entry, a deleted built-in entry, an added exception; names of days.
    run(&mut s, "tools.autocorrect", json!({"add": {"from": "wc", "to": "WordCraft"}}));
    run(&mut s, "tools.autocorrect", json!({"delete": "teh"}));
    run(&mut s, "tools.autocorrect", json!({"addException": "approx."}));
    typ(&mut s, "wc ships on monday. teh end is approx. three weeks ");
    assert_eq!(s.doc.plain_text(StoryRef::Body), "WordCraft ships on Monday. Teh end is approx. three weeks ");
    // AutoFormat options off: straight quotes, no dashes, no list; sentences not capitalised.
    let mut s = self::s();
    run(&mut s, "tools.autocorrect", json!({"smartQuotes": false, "dashes": false, "bullets": false, "capSentences": false, "capDays": false}));
    typ(&mut s, "* \"hi\" a -- b sunday ");
    assert_eq!(s.doc.plain_text(StoryRef::Body), "* \"hi\" a -- b sunday ");
    assert_eq!(paras(&s)[0].1, None);
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
    let s = typed("a -- b and c - d and e--f and café--g ");
    assert_eq!(s.doc.plain_text(StoryRef::Body), "A – b and c – d and e—f and café—g ");
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
fn undo_right_after_a_border_line_brings_the_characters_back() {
    let mut s = typed("Above\n---\n");
    assert_eq!(s.doc.plain_text(StoryRef::Body), "Above\n");
    run(&mut s, "edit.undo", json!({}));
    // The AutoFormat is undone on its own: the characters are back, the Enter stays.
    assert_eq!(s.doc.plain_text(StoryRef::Body), "Above\n---\n");
    assert!(s.doc.body.iter().filter_map(|b| b.as_para()).all(|p| p.props.borders.is_none()));
    assert_eq!(s.sel.focus, Pos::body(2, 0));
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(s.doc.plain_text(StoryRef::Body), "Above\n---");
}

#[test]
fn plus_and_hyphen_line_becomes_a_table() {
    let mut s = typed("+--+------+\nA");
    let Some(wordcraft_doc::Block::Table(t)) = s.doc.body.first().map(|b| &**b) else { panic!("no table: {:?}", paras(&s)) };
    assert_eq!(t.rows.len(), 1);
    assert_eq!(t.grid.len(), 2);
    assert!((t.grid[1] / t.grid[0] - 3.0).abs() < 0.01, "widths follow the hyphens: {:?}", t.grid);
    assert_eq!(t.rows[0].cells[0].blocks[0].as_para().map(|p| p.text.as_str()), Some("A"), "the caret is in the first cell");
    assert!(s.doc.body.get(1).and_then(|b| b.as_para()).is_some_and(|p| p.is_empty()), "a paragraph follows the table");
    // Undo right after brings the typed characters back, the Enter stays.
    run(&mut s, "edit.undo", json!({}));
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(s.doc.plain_text(StoryRef::Body), "+--+------+\n");
    assert!(s.doc.body.iter().all(|b| b.as_para().is_some()));
    // Not a table: text around it, or more columns than a table can hold.
    let s = typed("x +--+--+\n");
    assert_eq!(s.doc.plain_text(StoryRef::Body), "X +--+--+\n");
    let s = typed(&format!("+{}\n", "-+".repeat(64)));
    assert!(s.doc.body.iter().all(|b| b.as_para().is_some()));
}

#[test]
fn arrow_bullets() {
    for (keys, bullet) in [("> ", "\u{27A2}"), ("-> ", "\u{2794}"), ("=> ", "\u{21E8}")] {
        let s = typed(&format!("{keys}item\nnext"));
        assert_eq!(paras(&s), vec![para("Item", Some(0)), para("next", Some(0))], "{keys}");
        assert_eq!(label_at(&s, 0), bullet, "{keys}");
    }
    // An arrow list doesn't join a plain bullet list.
    let s = typed("* a\n\n> b");
    assert_eq!(label_at(&s, 0), "\u{2022}");
    assert_eq!(label_at(&s, 1), "\u{27A2}");
}

#[test]
fn smileys_are_replaced_inside_words_too() {
    let s = typed("so ok:) sad:( meh:| :) fine:-) ");
    assert_eq!(s.doc.plain_text(StoryRef::Body), "So ok\u{263A} sad\u{2639} meh\u{1F610} \u{263A} fine\u{263A} ");
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

/// Linked text in each body paragraph, and whether its paragraph mark carries a link.
fn links(s: &Session) -> Vec<(String, bool)> {
    s.doc
        .body
        .iter()
        .filter_map(|b| b.as_para())
        .map(|p| {
            let linked = p.run_ranges().filter(|(_, c)| c.link.is_some()).filter_map(|(r, _)| p.text.get(r)).collect();
            (linked, p.mark.link.is_some())
        })
        .collect()
}

#[test]
fn a_link_covers_only_its_own_text() {
    // Issue #413: linking a whole paragraph also linked its mark, so Enter carried the link into
    // the next paragraph and everything typed there (or hovered below it) became the link.
    let mut s = typed("hi\nhello");
    run(&mut s, "select.text", json!({"text": "hello"}));
    run(&mut s, "insert.link", json!({"url": "https://example.org/"}));
    run(&mut s, "caret.set", json!({"pos": {"block": 1, "off": 5}}));
    typ(&mut s, "\nnext");
    run(&mut s, "caret.set", json!({"pos": {"block": 1, "off": 0}}));
    typ(&mut s, "\n");
    run(&mut s, "caret.set", json!({"pos": {"block": 1, "off": 0}}));
    typ(&mut s, "above");
    assert_eq!(texts(&s), ["hi", "above", "hello", "next"]);
    let unlinked = (String::new(), false);
    assert_eq!(links(&s), [unlinked.clone(), unlinked.clone(), ("Hello".into(), false), unlinked]);
}

#[test]
fn typing_at_either_end_of_a_link_is_not_linked() {
    // As in Word: a link typed in with Ctrl+K, then text typed after it, at its end later on,
    // at its start, or after Enter, stays outside the link; typing inside it extends it.
    let mut s = typed("see ");
    run(&mut s, "insert.link", json!({"url": "https://example.org/", "text": "site"}));
    typ(&mut s, " now");
    run(&mut s, "caret.set", json!({"pos": {"block": 0, "off": 8}}));
    typ(&mut s, "!");
    run(&mut s, "caret.set", json!({"pos": {"block": 0, "off": 4}}));
    typ(&mut s, "a ");
    run(&mut s, "caret.set", json!({"pos": {"block": 0, "off": 8}}));
    typ(&mut s, "x");
    run(&mut s, "caret.set", json!({"pos": {"block": 0, "off": 11}}));
    typ(&mut s, "\nmore");
    assert_eq!(texts(&s), ["see a sixte", "more! now"]);
    assert_eq!(links(&s), [("sixte".into(), false), (String::new(), false)]);
}

#[test]
fn typing_then_selecting_then_typing_are_two_undo_steps() {
    // #419: a selection that moves the insertion point ends the typing group, like a caret move.
    let mut s = typed("alpha");
    run(&mut s, "select.text", json!({"text": "ph"}));
    run(&mut s, "text.insert", json!({"text": "X"}));
    assert_eq!(s.doc.para_at(&Pos::body(0, 0)).unwrap().text, "alXa");
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(s.doc.para_at(&Pos::body(0, 0)).unwrap().text, "alpha");
    assert_eq!(s.selected_text(), "ph");
    assert!(s.can_undo());
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(s.doc.para_at(&Pos::body(0, 0)).unwrap().text, "");
    // Uninterrupted typing (a read-only command in between included) is still one step.
    let mut s = typed("alpha");
    run(&mut s, "document.inspect", json!({}));
    typ(&mut s, "X");
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(s.doc.para_at(&Pos::body(0, 0)).unwrap().text, "");
}

#[test]
fn undoing_bold_at_the_caret_turns_bold_typing_off_again() {
    // #424: Bold at a caret in an empty paragraph, Undo, type: the text isn't bold.
    let mut s = s();
    run(&mut s, "format.bold", json!({"value": true}));
    run(&mut s, "edit.undo", json!({}));
    assert_ne!(s.typing_props().bold, Some(true));
    typ(&mut s, "abc");
    let p = s.doc.para_at(&Pos::body(0, 0)).unwrap();
    assert_eq!(p.text, "abc");
    assert_ne!(p.props_of_char(0).bold, Some(true));
    // Redo brings bold typing back.
    let mut s = Session::new(wordcraft_doc::Document::new());
    run(&mut s, "format.bold", json!({"value": true}));
    run(&mut s, "edit.undo", json!({}));
    run(&mut s, "edit.redo", json!({}));
    typ(&mut s, "abc");
    assert_eq!(s.doc.para_at(&Pos::body(0, 0)).unwrap().props_of_char(0).bold, Some(true));
}

/// Myanmar (#604): typed in memory order (consonant, medial, then the prevowel ေ that is drawn
/// before it), the caret steps over whole syllables and Backspace removes one sign at a time.
#[test]
fn myanmar_typing_steps_by_syllable_and_backspaces_by_sign() {
    // ကြေး ("money"): က + ြ + ေ + း, then မြန်မာ.
    let word = "\u{1000}\u{103C}\u{1031}\u{1038}";
    let mut s = typed(word);
    assert_eq!(paras(&s), vec![para(word, None)]);
    // Backspace takes the visarga, then the vowel sign: the consonant and medial stay.
    typ(&mut s, "\u{8}");
    assert_eq!(paras(&s)[0].0, "\u{1000}\u{103C}\u{1031}");
    typ(&mut s, "\u{8}\u{1031}\u{1038}");
    assert_eq!(paras(&s)[0].0, word, "retyped");
    // Left from the end skips the whole syllable (the visarga and aa signs aren't grapheme
    // extenders, so a grapheme step would stop inside it).
    let mut s = typed("\u{1019}\u{102C}\u{1005}\u{102C}"); // မာစာ
    run(&mut s, "caret.left", json!({}));
    assert_eq!(s.sel.focus.off, 6);
    run(&mut s, "caret.left", json!({}));
    assert_eq!(s.sel.focus.off, 0);
    run(&mut s, "caret.right", json!({}));
    assert_eq!(s.sel.focus.off, 6);
    // Delete removes the whole syllable after the caret.
    run(&mut s, "text.delete", json!({}));
    assert_eq!(paras(&s)[0].0, "\u{1019}\u{102C}");
}

/// Devanagari and Khmer: Backspace removes the last character (a matra), not the conjunct.
#[test]
fn indic_and_khmer_backspace_removes_one_character() {
    let mut s = typed("\u{0915}\u{094D}\u{0937}\u{093F}\u{8}"); // क्षि ⌫
    assert_eq!(paras(&s)[0].0, "\u{0915}\u{094D}\u{0937}");
    typ(&mut s, "\u{8}");
    assert_eq!(paras(&s)[0].0, "\u{0915}\u{094D}");
    let s = typed("\u{1781}\u{17D2}\u{1798}\u{17C2}\u{8}"); // ខ្មែ ⌫
    assert_eq!(paras(&s)[0].0, "\u{1781}\u{17D2}\u{1798}");
    // Latin keeps grapheme Backspace: e + combining acute goes as one.
    let s = typed("ae\u{301}\u{8}");
    assert_eq!(paras(&s)[0].0, "a");
}

/// Press a shortcut the way the keyboard does: run the command bound to it.
fn press(s: &mut Session, combo: &str) -> Result<serde_json::Value, crate::CmdError> {
    let id = s.registry.by_shortcut(combo).map(|c| c.id).unwrap_or_else(|| panic!("nothing bound to {combo}"));
    s.run(id, &json!({}))
}

/// Word's outline keys: Alt+Shift+arrows, ⌃⇧+arrows on macOS.
fn outline_key(arrow: &str) -> String {
    if cfg!(target_os = "macos") { format!("Ctrl+Shift+{arrow}") } else { format!("Alt+Shift+{arrow}") }
}

#[test]
fn alt_shift_down_then_up_moves_a_paragraph_and_back_and_undoes() {
    let mut s = typed("One\nTwo\nThree");
    run(&mut s, "caret.docStart", json!({}));
    press(&mut s, &outline_key("Down")).unwrap();
    assert_eq!(texts(&s), ["two", "one", "three"]);
    assert_eq!(s.sel.focus, Pos::body(1, 0), "the caret moves with the paragraph");
    press(&mut s, &outline_key("Up")).unwrap();
    assert_eq!(texts(&s), ["one", "two", "three"]);
    assert!(press(&mut s, &outline_key("Up")).is_err(), "nothing above the first paragraph");
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(texts(&s), ["two", "one", "three"], "each move is one undo step");
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(texts(&s), ["one", "two", "three"]);
    // A selected paragraph (with its mark) moves alone; list items keep their list.
    let mut s = typed("1. First\nSecond\n\nAfter");
    run(&mut s, "select.paragraph", json!({}));
    press(&mut s, &outline_key("Up")).unwrap();
    assert_eq!(paras(&s), vec![para("First", Some(0)), para("After", None), para("Second", Some(0))]);
    assert!(press(&mut s, &outline_key("Down")).is_ok());
    assert!(press(&mut s, &outline_key("Down")).is_err(), "nothing below the last paragraph");
}

fn style_of(s: &Session, i: usize) -> Option<String> {
    s.doc.body.get(i).and_then(|b| b.as_para()).and_then(|p| p.props.style.clone())
}

#[test]
fn alt_shift_left_and_right_promote_and_demote_headings_and_list_items() {
    let mut s = typed("Intro\nBody");
    run(&mut s, "caret.docStart", json!({}));
    run(&mut s, "para.heading2", json!({}));
    run(&mut s, "caret.docEnd", json!({}));
    // Promoted body text takes the level of the heading before it; then one level at a time.
    press(&mut s, &outline_key("Left")).unwrap();
    assert_eq!(style_of(&s, 1).as_deref(), Some("Heading2"));
    press(&mut s, &outline_key("Right")).unwrap();
    assert_eq!(style_of(&s, 1).as_deref(), Some("Heading3"));
    press(&mut s, &outline_key("Left")).unwrap();
    press(&mut s, &outline_key("Left")).unwrap();
    assert_eq!(style_of(&s, 1).as_deref(), Some("Heading1"));
    assert!(press(&mut s, &outline_key("Left")).is_err(), "Heading 1 is the top level");
    for _ in 0..8 {
        press(&mut s, &outline_key("Right")).unwrap();
    }
    assert_eq!(style_of(&s, 1).as_deref(), Some("Heading9"));
    assert!(press(&mut s, &outline_key("Right")).is_err(), "Heading 9 is the lowest heading");
    // In a list they change the item's level, like Tab and Shift+Tab.
    let mut s = typed("* One\nTwo");
    press(&mut s, &outline_key("Right")).unwrap();
    assert_eq!(paras(&s), vec![para("One", Some(0)), para("Two", Some(1))]);
    press(&mut s, &outline_key("Left")).unwrap();
    assert_eq!(paras(&s), vec![para("One", Some(0)), para("Two", Some(0))]);
}

#[test]
fn down_and_up_from_a_line_start_beside_a_table_go_into_it() {
    let mut s = typed("Intro line\n");
    run(&mut s, "insert.table", json!({"rows": 2, "cols": 2}));
    let (table, _, _) = s.sel.focus.path.cell().unwrap();
    typ(&mut s, "a");
    let cell = |r: u32, c: u32| {
        let mut p = table.0.clone();
        p.extend([r, c, 0]);
        wordcraft_doc::Path(p)
    };
    // From the start of the line above, left of the cells' text: the first cell, then the one
    // below, not the paragraph after the table.
    run(&mut s, "caret.docStart", json!({}));
    run(&mut s, "caret.down", json!({}));
    assert_eq!(s.sel.focus.path, cell(0, 0));
    run(&mut s, "caret.down", json!({}));
    assert_eq!(s.sel.focus.path, cell(1, 0), "then the cell below");
    // Up from the start of the paragraph after it: the last row's first cell.
    run(&mut s, "caret.docEnd", json!({}));
    run(&mut s, "caret.up", json!({}));
    assert_eq!(s.sel.focus.path, cell(1, 0));
    // Shift+Down from the line above takes the first row, not the whole table.
    run(&mut s, "caret.docStart", json!({}));
    run(&mut s, "caret.down", json!({"extend": true}));
    assert_eq!(s.sel.focus.path, cell(0, 1));
}

#[test]
fn alt_x_toggles_between_a_character_and_its_code() {
    if !cfg!(target_os = "macos") {
        assert_eq!(s().registry.by_shortcut("Alt+X").map(|c| c.id), Some("text.toggleUnicode"));
    }
    let mut s = s();
    run(&mut s, "text.insert", json!({"text": "x 00e9"}));
    run(&mut s, "text.toggleUnicode", json!({}));
    assert_eq!(texts(&s), ["x é"]);
    run(&mut s, "text.toggleUnicode", json!({}));
    assert_eq!(texts(&s), ["x 00e9"]);
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(texts(&s), ["x é"], "one undo step");
    run(&mut s, "text.insert", json!({"text": " U+1F600"}));
    run(&mut s, "text.toggleUnicode", json!({}));
    assert_eq!(texts(&s), ["x é 😀"]);
    // A selected code converts too.
    run(&mut s, "text.insert", json!({"text": " 2014"}));
    run(&mut s, "select.text", json!({"text": "2014"}));
    run(&mut s, "text.toggleUnicode", json!({}));
    assert_eq!(texts(&s), ["x é 😀 —"]);
}

#[test]
fn alt_numpad_codes_map_like_word_on_windows() {
    use crate::cmd::text::{alt_code_char, oem_code_page};
    let c = |code| alt_code_char(code, 1252, 437);
    // Leading 0: Windows-1252.
    assert_eq!(c("0233"), Some('é'));
    assert_eq!(c("0128"), Some('€'));
    assert_eq!(c("0150"), Some('–'));
    assert_eq!(c("065"), Some('A'));
    assert_eq!(c("0032"), Some(' '));
    assert_eq!(c("0255"), Some('ÿ'));
    // No leading 0: the OEM code page (437 here, 850 in Western Europe).
    assert_eq!(c("233"), Some('Θ'));
    // The issue said 233 → é in 850; the CP850 mapping file has Ú at 0xE9 (é is 130 in both).
    assert_eq!(alt_code_char("233", 1252, 850), Some('Ú'));
    assert_eq!(alt_code_char("130", 1252, 850), Some('é'));
    assert_eq!(c("130"), Some('é'));
    assert_eq!(alt_code_char("155", 1252, 850), Some('ø'));
    assert_eq!(c("155"), Some('¢'));
    assert_eq!(c("65"), Some('A'));
    assert_eq!(c("1"), Some('☺'));
    assert_eq!(c("7"), Some('•'));
    assert_eq!(c("127"), Some('⌂'));
    assert_eq!(c("176"), Some('░'));
    // Above 255: the Unicode character with that decimal value.
    assert_eq!(c("8364"), Some('€'));
    assert_eq!(c("09733"), Some('★'));
    assert_eq!(c("128512"), Some('😀'));
    // Nothing for junk, 0, control characters, surrogates or values past Unicode.
    for bad in ["", "0", "000", "0009", "0013", "0127", "0129", "x1", "-5", "55296", "1114112", "99999999999999999999", "4294967361"] {
        assert_eq!(c(bad), None, "{bad:?}");
    }
    assert_eq!(oem_code_page(None), 437);
    assert_eq!(oem_code_page(Some("en-US")), 437);
    assert_eq!(oem_code_page(Some("en_GB.UTF-8")), 850);
    assert_eq!(oem_code_page(Some("fr-FR")), 850);
    assert_eq!(oem_code_page(Some("de")), 850);
    assert_eq!(oem_code_page(Some("ru-RU")), 437, "only 437/850 without the shell's code pages");
}

#[test]
fn alt_numpad_codes_follow_the_system_code_pages() {
    use crate::cmd::text::{alt_code_char, windows_code_pages};
    // Polish (1250/852): Alt+0185 and Alt+165 are both ą, not ¹ / Ñ.
    assert_eq!(alt_code_char("0185", 1250, 852), Some('ą'));
    assert_eq!(alt_code_char("165", 1250, 852), Some('ą'));
    assert_eq!(alt_code_char("0185", 1252, 437), Some('¹'));
    // Russian (1251/866), Greek (1253/737), Turkish (1254/857).
    assert_eq!(alt_code_char("0207", 1251, 866), Some('П'));
    assert_eq!(alt_code_char("143", 1251, 866), Some('П'));
    assert_eq!(alt_code_char("0193", 1253, 737), Some('Α'));
    assert_eq!(alt_code_char("128", 1253, 737), Some('Α'));
    assert_eq!(alt_code_char("0253", 1254, 857), Some('ı'));
    assert_eq!(alt_code_char("141", 1254, 857), Some('ı'));
    // Above 255 the code page doesn't matter.
    assert_eq!(alt_code_char("8364", 1251, 866), Some('€'));
    // A byte the code page leaves unassigned types nothing.
    assert_eq!(alt_code_char("213", 1254, 857), None);
    assert_eq!(alt_code_char("0219", 874, 437), None);
    assert_eq!(windows_code_pages(None), (1252, 437));
    assert_eq!(windows_code_pages(Some("en-US")), (1252, 437));
    assert_eq!(windows_code_pages(Some("de-DE")), (1252, 850));
    assert_eq!(windows_code_pages(Some("pl-PL")), (1250, 852));
    assert_eq!(windows_code_pages(Some("sr-Latn-RS")), (1250, 852));
    assert_eq!(windows_code_pages(Some("ru_RU.UTF-8")), (1251, 866));
    assert_eq!(windows_code_pages(Some("el-GR")), (1253, 737));
    assert_eq!(windows_code_pages(Some("tr-TR")), (1254, 857));
    assert_eq!(windows_code_pages(Some("ja-JP")), (1252, 437));
}

#[test]
fn alt_numpad_code_types_one_character() {
    // `texts` lowercases; the capitals matter here.
    let raw = |s: &Session| paras(s).into_iter().map(|p| p.0).collect::<Vec<_>>();
    let mut s = s();
    run(&mut s, "text.insert", json!({"text": "caf"}));
    run(&mut s, "text.typeCode", json!({"code": "0233"}));
    run(&mut s, "text.insert", json!({"text": " "}));
    run(&mut s, "text.typeCode", json!({"code": "233", "oem": 437}));
    run(&mut s, "text.insert", json!({"text": " "}));
    run(&mut s, "text.typeCode", json!({"code": "155", "oem": 850}));
    assert_eq!(raw(&s), ["Café Θ ø"]);
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(raw(&s), ["Café Θ "], "one undo step per code");
    assert!(s.run("text.typeCode", &json!({"code": "0"})).is_err());
    assert!(s.run("text.typeCode", &json!({"code": "65", "oem": 855})).is_err());
    assert!(s.run("text.typeCode", &json!({"code": "065", "ansi": 932})).is_err());
    assert!(s.run("text.typeCode", &json!({"code": "065", "ansi": 70000})).is_err());
    assert_eq!(raw(&s), ["Café Θ "]);
    // The system's code pages: Polish Alt+0185 and Alt+165 both type ą.
    run(&mut s, "text.typeCode", json!({"code": "0185", "ansi": 1250, "oem": 852}));
    run(&mut s, "text.typeCode", json!({"code": "165", "ansi": 1250, "oem": 852}));
    run(&mut s, "text.typeCode", json!({"code": "143", "oem": 866}));
    assert_eq!(raw(&s), ["Café Θ ąąП"]);
}

#[test]
fn shift_f5_goes_back_through_the_last_edits() {
    let mut s = typed("One\nTwo\nThree");
    run(&mut s, "caret.docStart", json!({}));
    let mut back = || {
        press(&mut s, "Shift+F5").unwrap();
        s.sel.focus.clone()
    };
    assert_eq!(back(), Pos::body(2, 5), "the last edit first");
    assert_eq!(back(), Pos::body(1, 3));
    assert_eq!(back(), Pos::body(0, 3));
    assert_eq!(back(), Pos::body(2, 5), "then round again");
    assert!(Session::new(wordcraft_doc::Document::new()).run("edit.goBack", &json!({})).is_err(), "nothing edited yet");
}

#[test]
fn shift_right_at_the_end_of_a_cell_selects_the_cell_first() {
    let mut s = s();
    run(&mut s, "insert.table", json!({"rows": 2, "cols": 2}));
    let (table, _, _) = s.sel.focus.path.cell().unwrap();
    typ(&mut s, "ab");
    run(&mut s, "caret.left", json!({}));
    let shift_right = |s: &mut Session| {
        run(s, "caret.right", json!({"extend": true}));
        s.cell_range().map(|r| (r.rows, r.cols))
    };
    assert_eq!(shift_right(&mut s), None, "the letter first");
    assert_eq!((s.sel.anchor.off, s.sel.focus.off), (1, 2));
    assert_eq!(shift_right(&mut s), Some(((0, 0), (0, 0))), "then the cell alone");
    assert_eq!(shift_right(&mut s), Some(((0, 0), (0, 1))), "then the next cell with it");
    // In an empty cell the first press takes it.
    let mut empty = table.0.clone();
    empty.extend([1, 0, 0]);
    let pos = Pos { story: StoryRef::Body, path: wordcraft_doc::Path(empty), off: 0 };
    run(&mut s, "caret.set", json!({"pos": pos}));
    assert_eq!(shift_right(&mut s), Some(((1, 1), (0, 0))));
}

#[test]
fn two_initial_capitals_are_corrected() {
    let s = typed("the WOrd and THe end, not IDs or CDs. TV is fine ");
    assert_eq!(s.doc.plain_text(StoryRef::Body), "The Word and The end, not IDs or CDs. TV is fine ");
    // "TEh" loses its second capital, then the replacement list fixes the typo.
    assert_eq!(typed("TEh ").doc.plain_text(StoryRef::Body), "The ");
    // A user exception.
    let mut s = typed("");
    run(&mut s, "tools.autocorrect", json!({"addInitialCapsException": "WCraft"}));
    typ(&mut s, "WCraft ");
    assert_eq!(s.doc.plain_text(StoryRef::Body), "WCraft ");
}

#[test]
fn accidental_caps_lock_is_corrected() {
    let s = typed("a wORD and hELLO there. iOS stays ");
    assert_eq!(s.doc.plain_text(StoryRef::Body), "A Word and Hello there. iOS stays ");
}

#[test]
fn capitalisation_fixes_undo_and_turn_off() {
    let mut s = typed("WOrd");
    typ(&mut s, " ");
    assert_eq!(s.doc.plain_text(StoryRef::Body), "Word ");
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(s.doc.plain_text(StoryRef::Body), "WOrd ");
    let mut s = typed("x wORD");
    typ(&mut s, " ");
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(s.doc.plain_text(StoryRef::Body), "X wORD ");
    let mut s = typed("");
    run(&mut s, "tools.autocorrect", json!({"twoInitialCaps": false, "capsLock": false}));
    typ(&mut s, "x WOrd wORD ");
    assert_eq!(s.doc.plain_text(StoryRef::Body), "X WOrd wORD ");
}

#[test]
fn first_letter_of_a_table_cell_is_capitalised() {
    let cell_text = |s: &Session, c: usize| {
        s.doc.body.iter().find_map(|b| b.as_table()).and_then(|t| t.rows.first()?.cells.get(c)?.blocks.first()?.as_para().map(|p| p.text.clone()))
    };
    let mut s = typed("");
    run(&mut s, "insert.table", json!({"rows": 1, "cols": 2}));
    typ(&mut s, "apples and pears\tplums ");
    assert_eq!(cell_text(&s, 0).as_deref(), Some("Apples and pears"));
    assert_eq!(cell_text(&s, 1).as_deref(), Some("Plums "));
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(cell_text(&s, 1).as_deref(), Some("plums "));
    let mut s = typed("");
    run(&mut s, "tools.autocorrect", json!({"capCells": false}));
    run(&mut s, "insert.table", json!({"rows": 1, "cols": 1}));
    typ(&mut s, "plums ");
    assert_eq!(cell_text(&s, 0).as_deref(), Some("plums "));
}

/// File › Options › Advanced, "Typing replaces selected text" (#485): on (Word's default) the
/// typed text replaces the selection; off, it goes in front of the selection and keeps it.
#[test]
fn typing_over_a_selection_follows_the_replace_selection_option() {
    let mut s = typed("One two");
    run(&mut s, "select.text", json!({"text": "two"}));
    typ(&mut s, "X");
    assert_eq!(s.doc.plain_text(StoryRef::Body), "One X");
    let mut s = typed("One two");
    run(&mut s, "file.options", json!({"typingReplacesSelection": false}));
    run(&mut s, "select.text", json!({"text": "two"}));
    typ(&mut s, "XY");
    assert_eq!(s.doc.plain_text(StoryRef::Body), "One XYtwo");
}

/// Overtype (#485): typed characters replace the ones after the caret, never past the end of the
/// paragraph; the Insert key toggles it only with "Use the Insert key to control overtype" on
/// (the UI checks that option; `text.overtype` is the switch).
#[test]
fn overtype_replaces_the_characters_after_the_caret() {
    let mut s = typed("Abcdef\nXy");
    run(&mut s, "caret.set", json!({"pos": Pos::body(0, 1)}));
    run(&mut s, "text.overtype", json!({"value": true}));
    typ(&mut s, "XY");
    assert_eq!(s.doc.plain_text(StoryRef::Body), "AXYdef\nXy");
    run(&mut s, "caret.set", json!({"pos": Pos::body(0, 5)}));
    typ(&mut s, "123");
    assert_eq!(s.doc.plain_text(StoryRef::Body), "AXYde123\nXy", "the paragraph mark isn't typed over");
    run(&mut s, "text.overtype", json!({}));
    run(&mut s, "caret.set", json!({"pos": Pos::body(0, 0)}));
    typ(&mut s, "Z");
    assert_eq!(s.doc.plain_text(StoryRef::Body), "ZAXYde123\nXy", "insert mode again");
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

/// #713: Arabic or Hebrew typed into an empty paragraph makes it right-to-left; a paragraph that
/// already has letters, or whose style sets a direction, keeps its own.
#[test]
fn typing_rtl_letters_into_an_empty_paragraph_makes_it_rtl() {
    let bidi = |s: &Session, i: usize| s.doc.body.iter().filter_map(|b| b.as_para()).nth(i).and_then(|p| p.props.bidi);
    // Empty paragraph: the first Arabic letter flips it, the rest keep it.
    let s = typed("سلام");
    assert_eq!(bidi(&s, 0), Some(true));
    // Digits first, then Arabic: still no letters before, so it flips.
    let s = typed("1. سلام");
    assert_eq!(bidi(&s, 0), Some(true));
    // Latin first: Arabic typed later doesn't flip the paragraph.
    let s = typed("Hello سلام");
    assert_eq!(bidi(&s, 0), None);
    // A new paragraph after an RTL one starts its own detection.
    let s = typed("سلام\nHello");
    assert_eq!(bidi(&s, 1), Some(true), "Enter carries the paragraph's direction on, as in Word");
    // A style that sets left-to-right wins.
    let mut doc = Session::new(wordcraft_doc::Document::new());
    if let Some(st) = doc.doc.styles.get_mut("Normal") {
        st.para.bidi = Some(false);
    }
    typ(&mut doc, "سلام");
    assert_eq!(bidi(&doc, 0), None);
    // An opened paragraph without w:bidi lays out left-to-right whatever its text: the caret
    // after the Arabic word sits near the left margin, not the right one.
    let mut doc = typed("سلام");
    let path = doc.sel.focus.path.clone();
    if let Ok(p) = doc.doc.para_mut(StoryRef::Body, &path) {
        p.props.bidi = None;
        p.touch();
    }
    let x = doc.layout().caret(&doc.sel.focus).map(|c| c.x).unwrap_or(f32::MAX);
    assert!(x < 300.0, "left-to-right paragraph: caret at {x}");
}

/// Select All then Delete or Backspace empties the document, tables included (Word, #1198):
/// whether the text starts with a table or with a paragraph.
#[test]
fn select_all_then_delete_removes_tables() {
    let tables = |s: &Session| s.doc.body.iter().filter(|b| b.as_table().is_some()).count();
    // With the caret in a table, Select All grows the selection a step at a time (cell, row,
    // table, document; #1478): press it until the whole document is selected.
    let select_whole_document = |s: &mut Session| {
        for _ in 0..6 {
            let before = s.sel.clone();
            run(s, "select.all", json!({}));
            if s.sel == before {
                break;
            }
        }
    };
    for key in ["text.delete", "text.backspace"] {
        let mut s = typed("");
        run(&mut s, "insert.table", json!({"rows": 2, "cols": 2}));
        typ(&mut s, "cell");
        assert_eq!(tables(&s), 1);
        select_whole_document(&mut s);
        run(&mut s, key, json!({}));
        assert_eq!(tables(&s), 0, "{key}: a table at the start of the document goes");
        assert_eq!(s.doc.plain_text(StoryRef::Body), "", "{key}");

        let mut s = typed("before\n");
        run(&mut s, "insert.table", json!({"rows": 2, "cols": 2}));
        typ(&mut s, "cell");
        assert_eq!(tables(&s), 1);
        select_whole_document(&mut s);
        run(&mut s, key, json!({}));
        assert_eq!(tables(&s), 0, "{key}: a table after text goes");
        assert_eq!(s.doc.plain_text(StoryRef::Body), "", "{key}");
    }
}

/// #898: a click just after a citation at the end of a paragraph puts the caret before the
/// citation's hidden field end; what is typed there goes after the citation, as in Word, instead
/// of into its result (Zotero then reports the citation as edited by hand).
#[test]
fn typing_after_a_citation_goes_outside_its_field() {
    use wordcraft_doc::props::CharProps;
    use wordcraft_doc::{InlineObject, Path};
    let mut doc = wordcraft_doc::Document::from_text("Cats purr ");
    let c = CharProps::default();
    let p = doc.para_mut(StoryRef::Body, &Path::top(0)).unwrap();
    let start = p.len();
    p.insert_object(start, InlineObject::FieldStart { instr: "ADDIN ZOTERO_ITEM CSL_CITATION {}".into(), locked: false, dirty: false }, &c).unwrap();
    let n = p.insert_text(p.len(), "(Smith 2020)", &c).unwrap();
    p.insert_object(start + 3 + n, InlineObject::FieldEnd, &c).unwrap();
    let mut s = Session::new(doc);
    let end = Pos::body(0, s.doc.para(StoryRef::Body, &Path::top(0)).unwrap().len());
    let at = s.layout().caret(&end).unwrap();
    let hit = s.layout().hit(at.page, at.x + 20.0, at.top + at.height / 2.0, StoryRef::Body).unwrap();
    s.sel = crate::Selection::caret(hit);
    typ(&mut s, ". Dogs bark.");
    assert_eq!(s.doc.plain_text(StoryRef::Body), "Cats purr (Smith 2020). Dogs bark.");
    let f = s.doc.field_ranges(StoryRef::Body);
    assert_eq!(f.len(), 1);
    let inside = Pos { off: f[0].start.off + wordcraft_doc::para::OBJ.len_utf8(), ..f[0].start.clone() };
    assert_eq!(s.doc.copy_range(&inside, &f[0].end).plain_text(), "(Smith 2020)");
}

/// #1670: smart quotes in Hebrew follow the characters around the quote in logical order, as
/// Word does (Word for Mac 16.113.4): an opening “ after a space or at the start, a closing ”
/// after a letter, and ” inside an acronym (צה"ל), not gershayim; ' after a letter is ’.
#[test]
fn smart_quotes_in_hebrew_use_logical_order() {
    let bidi = |s: &Session| s.doc.body.iter().filter_map(|b| b.as_para()).next().and_then(|p| p.props.bidi);
    let s = typed("הוא אמר \"שלום\" לי");
    assert_eq!(bidi(&s), Some(true), "a right-to-left paragraph");
    assert_eq!(s.doc.plain_text(StoryRef::Body), "הוא אמר “שלום” לי");
    let s = typed("\"שלום\"");
    assert_eq!(s.doc.plain_text(StoryRef::Body), "“שלום”");
    let s = typed("צה\"ל ");
    assert_eq!(s.doc.plain_text(StoryRef::Body), "צה”ל ");
    let s = typed("ג'ירפה 'כן'");
    assert_eq!(s.doc.plain_text(StoryRef::Body), "ג’ירפה ‘כן’");
}
