//! Typing behaviour checked side by side against Word: keystrokes go through the same commands
//! as the keyboard (one `text.insert` per character, Enter, Tab, Backspace), and the expected
//! results are what Word produces for the same keys.

use serde_json::json;
use wordcraft_doc::props::{BorderStyle, VertAlign};
use wordcraft_doc::{Pos, StoryRef};

use crate::Session;

/// Create an empty session for command-driven typing fixtures.
fn s() -> Session {
    Session::new(wordcraft_doc::Document::new())
}

/// Execute a registry command and report its id on a test failure.
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

/// Expected paragraph text and list membership for typing assertions.
fn para(text: &str, level: Option<u8>) -> (String, Option<u8>, bool) {
    (text.to_string(), level, level.is_some())
}

/// Create a session and enter its fixture through the actual typing commands.
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

/// #2143: in a Word file whose list comes from the paragraph styles (List Number, List Number 2
/// with `w:numPr` in styles.xml, none on the paragraphs), Enter on an empty nested item moves it
/// up a level and then ends the list, as with direct numbering.
#[test]
fn enter_on_an_empty_item_numbered_by_its_style_moves_it_up_a_level() {
    use std::io::Write;
    let w = r#"xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main""#;
    let style = |id: &str, name: &str, ilvl: u8| {
        format!(
            r#"<w:style w:type="paragraph" w:styleId="{id}"><w:name w:val="{name}"/><w:pPr><w:numPr><w:ilvl w:val="{ilvl}"/><w:numId w:val="1"/></w:numPr></w:pPr></w:style>"#
        )
    };
    let styles = format!(
        r#"<w:styles {w}><w:style w:type="paragraph" w:default="1" w:styleId="Normal"><w:name w:val="Normal"/></w:style>{}{}</w:styles>"#,
        style("ListNumber", "List Number", 0),
        style("ListNumber2", "List Number 2", 1)
    );
    let numbering = format!(
        r#"<w:numbering {w}><w:abstractNum w:abstractNumId="0"><w:lvl w:ilvl="0"><w:start w:val="1"/><w:numFmt w:val="decimal"/><w:lvlText w:val="%1."/><w:pPr><w:ind w:left="720" w:hanging="360"/></w:pPr></w:lvl><w:lvl w:ilvl="1"><w:start w:val="1"/><w:numFmt w:val="lowerLetter"/><w:lvlText w:val="%2."/><w:pPr><w:ind w:left="1440" w:hanging="360"/></w:pPr></w:lvl></w:abstractNum><w:num w:numId="1"><w:abstractNumId w:val="0"/></w:num></w:numbering>"#
    );
    let p = |st: &str, text: &str| format!(r#"<w:p><w:pPr><w:pStyle w:val="{st}"/></w:pPr><w:r><w:t>{text}</w:t></w:r></w:p>"#);
    let body: String =
        [p("ListNumber", "Lorem"), p("ListNumber", "Ipsum"), p("ListNumber2", "Latin"), p("ListNumber2", "Nonsense"), p("ListNumber2", "")].concat();
    let document = format!(r#"<w:document {w}><w:body>{body}</w:body></w:document>"#);
    let rel = |id: &str, kind: &str, target: &str| {
        format!(r#"<Relationship Id="{id}" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/{kind}" Target="{target}"/>"#)
    };
    let rels_ns = r#"xmlns="http://schemas.openxmlformats.org/package/2006/relationships""#;
    let main = format!("<Relationships {rels_ns}>{}</Relationships>", rel("rId1", "officeDocument", "word/document.xml"));
    let doc_rels =
        format!("<Relationships {rels_ns}>{}{}</Relationships>", rel("rId1", "styles", "styles.xml"), rel("rId2", "numbering", "numbering.xml"));
    let ct = r#"<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="xml" ContentType="application/xml"/><Override PartName="/word/document.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml"/><Override PartName="/word/styles.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.styles+xml"/><Override PartName="/word/numbering.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.numbering+xml"/></Types>"#;
    let mut z = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    let o = zip::write::SimpleFileOptions::default();
    for (name, data) in [
        ("[Content_Types].xml", ct),
        ("_rels/.rels", main.as_str()),
        ("word/_rels/document.xml.rels", doc_rels.as_str()),
        ("word/document.xml", document.as_str()),
        ("word/styles.xml", styles.as_str()),
        ("word/numbering.xml", numbering.as_str()),
    ] {
        z.start_file(name, o).unwrap();
        z.write_all(data.as_bytes()).unwrap();
    }
    let doc = crate::io::open_bytes("t.docx", &z.finish().unwrap().into_inner()).unwrap();
    let mut s = Session::new(doc);
    // The list level each paragraph shows, wherever its numbering comes from.
    let levels = |s: &Session| -> Vec<Option<u8>> {
        s.doc
            .body
            .iter()
            .filter_map(|b| b.as_para())
            .map(|p| s.doc.styles.resolve_para(&p.props).numbering.filter(|n| n.num != 0).map(|n| n.level))
            .collect()
    };
    assert_eq!(levels(&s), [Some(0), Some(0), Some(1), Some(1), Some(1)]);
    s.sel = crate::Selection::caret(Pos::body(4, 0));
    typ(&mut s, "\n");
    assert_eq!(levels(&s), [Some(0), Some(0), Some(1), Some(1), Some(0)]);
    assert_eq!(s.sel.focus, Pos::body(4, 0));
    typ(&mut s, "\n");
    assert_eq!(levels(&s), [Some(0), Some(0), Some(1), Some(1), None]);
    run(&mut s, "edit.undo", json!({}));
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(levels(&s), [Some(0), Some(0), Some(1), Some(1), Some(1)]);
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

/// #3352 follow-up (CodeRabbit): a heading numbered only through its style's linked Multilevel
/// List (#1793) — `props.numbering` itself unset — isn't papered over by a fresh bullet when "- "
/// is typed at its start; the trigger stays as plain text and the heading's own number is kept.
#[test]
fn list_autoformat_does_not_override_a_style_linked_heading_number() {
    let mut s = s();
    typ(&mut s, "One");
    run(&mut s, "para.heading1", json!({}));
    run(&mut s, "list.multilevelPreset", json!({"preset": "headings"}));
    typ(&mut s, "\nTwo");
    run(&mut s, "para.heading2", json!({}));
    assert_eq!(numbers(&mut s), [num(Some("1"), "Heading1"), num(Some("1.1"), "Heading2")]);
    assert_eq!(s.doc.para_at(&Pos::body(1, 0)).unwrap().props.numbering, None, "numbered through the style, not directly");
    run(&mut s, "caret.home", json!({}));
    typ(&mut s, "- ");
    assert_eq!(numbers(&mut s), [num(Some("1"), "Heading1"), num(Some("1.1"), "Heading2")], "the heading number survives");
    assert_eq!(paras(&s)[1].0, "- Two", "the trigger stays as typed text, not stripped into a bullet");
}

/// A heading numbered only through its style's linked Multilevel List keeps "---" and "+--+--+"
/// as text on Enter, as a paragraph numbered directly does: no border, no table, and at the top
/// of the document the heading keeps its number.
#[test]
fn border_and_table_lines_stay_text_in_a_style_numbered_heading() {
    for (above, keys) in [("", "---"), ("Above\n", "---"), ("", "+--+--+"), ("Above\n", "+--+--+")] {
        let mut s = s();
        typ(&mut s, &format!("{above}{keys}"));
        run(&mut s, "para.heading1", json!({}));
        run(&mut s, "list.multilevelPreset", json!({"preset": "headings"}));
        let at = Pos::body(s.doc.body.len() - 1, 0);
        assert_eq!(s.doc.para_at(&at).unwrap().props.numbering, None, "numbered through the style, not directly");
        typ(&mut s, "\nText");
        assert_eq!(s.doc.plain_text(StoryRef::Body), format!("{above}{keys}\nText"), "{above:?}{keys}");
        assert!(s.doc.body.iter().all(|b| b.as_para().is_some_and(|p| p.props.borders.is_none())), "{above:?}{keys}");
        let mut want = vec![num(Some("1"), "Heading1"), num(None, "Normal")];
        if !above.is_empty() {
            want.insert(0, num(None, "Normal"));
        }
        assert_eq!(numbers(&mut s), want, "{above:?}{keys}");
    }
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

/// #3352: typing the trigger at the start of an already-written paragraph (not only a fresh,
/// empty one) still turns it into a list item, keeping the rest of the paragraph's text — as in
/// Word, confirmed in Word for Windows 2026-10-21.
#[test]
fn list_autoformat_triggers_in_front_of_existing_text() {
    let mut s = s();
    typ(&mut s, "On peut faire mieux");
    run(&mut s, "caret.home", json!({}));
    typ(&mut s, "- ");
    assert_eq!(paras(&s), vec![para("On peut faire mieux", Some(0))]);
    assert_eq!(label_at(&s, 0), "-");
    // Undo right after takes only the AutoFormat back, as for a fresh paragraph.
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(paras(&s), vec![("- On peut faire mieux".to_string(), None, false)]);
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
fn smart_quotes_nested_and_opening_delimiters() {
    let s = typed("\"'nested'\" '\"inner\"' (\"'parens'\")");
    assert_eq!(s.doc.plain_text(StoryRef::Body), "“‘nested’” ‘“inner”’ (“‘parens’”)");
    // Empty quotes preserve open then close
    let s = typed("\"\" ''");
    assert_eq!(s.doc.plain_text(StoryRef::Body), "“” ‘’");
    // After opening guillemets and low quotes
    let s = typed("«'french'» „'german'“");
    assert_eq!(s.doc.plain_text(StoryRef::Body), "«‘french’» „‘german’“");
}

/// #2223: the double-hyphen arrow is a replacement, not the existing arrow bullet.
#[test]
fn double_hyphen_arrow_autocorrect_preserves_typing_scope_and_undo() {
    let mut s = typed("go --> ");
    assert_eq!(s.doc.plain_text(StoryRef::Body), "Go → ");
    assert_eq!(paras(&s)[0].1, None);
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(s.doc.plain_text(StoryRef::Body), "Go --> ");
    let s = typed("go --> here");
    assert_eq!(s.doc.plain_text(StoryRef::Body), "Go → here");
    let s = typed("--> ");
    assert_eq!(s.doc.plain_text(StoryRef::Body), "→ ");
    assert_eq!(paras(&s)[0].1, None);
}

#[test]
fn double_hyphen_arrow_respects_autocorrect_preferences() {
    for prefs in [json!({"enabled": false}), json!({"replaceText": false}), json!({"delete": "-->"})] {
        let mut s = s();
        run(&mut s, "tools.autocorrect", prefs);
        typ(&mut s, "go --> here");
        assert!(s.doc.plain_text(StoryRef::Body).ends_with(" --> here"));
    }
    let mut s = s();
    run(&mut s, "review.language", json!({"lang": "pl-PL", "default": true}));
    typ(&mut s, "go --> here");
    assert_eq!(s.doc.plain_text(StoryRef::Body), "Go → here");
    let mut s = self::s();
    run(&mut s, "tools.autocorrect", json!({"add": {"from": "-->", "to": "custom"}}));
    typ(&mut s, "go --> here");
    assert_eq!(s.doc.plain_text(StoryRef::Body), "Go custom here");
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

/// A word after a no-break space or an ideographic space is corrected as after a plain space:
/// replacements, sentence capitals, ordinals and links.
#[test]
fn autocorrect_finds_the_word_after_a_wide_space() {
    let s = typed("a\u{a0}teh x\u{3000}teh ");
    assert_eq!(s.doc.plain_text(StoryRef::Body), "a\u{a0}the x\u{3000}the ");
    let s = typed("end.\u{a0}hello end.\u{3000}world ");
    assert_eq!(s.doc.plain_text(StoryRef::Body), "End.\u{a0}Hello end.\u{3000}World ");
    let s = typed("the\u{a0}1st and\u{3000}22nd ");
    let p = s.doc.para_at(&Pos::body(0, 0)).unwrap();
    let sup = |t: &str| p.props_of_char(p.text.find(t).unwrap() + t.len() - 2).vert_align == Some(VertAlign::Superscript);
    assert!(sup("1st") && sup("22nd"));
    let s = typed("see\u{a0}https://example.org/log, and\u{3000}https://example.net/day, then ");
    let p = s.doc.para_at(&Pos::body(0, 0)).unwrap();
    let link = |t: &str| p.props_of_char(p.text.find(t).unwrap()).link.clone();
    assert_eq!(link("https://example.org").as_deref(), Some("https://example.org/log"));
    assert_eq!(link("https://example.net").as_deref(), Some("https://example.net/day"));
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
fn bold_and_italic_marks_become_formatting() {
    let mut s = s();
    run(&mut s, "tools.autocorrect", json!({"boldItalic": true, "capSentences": false}));
    typ(&mut s, "say *very* nice, _quite_ good *two words*. ok\n*end*\nnext");
    assert_eq!(s.doc.plain_text(StoryRef::Body), "say very nice, quite good two words. ok\nend\nnext");
    let p = s.doc.para_at(&Pos::body(0, 0)).unwrap();
    let at = |t: &str| p.props_of_char(p.text.find(t).unwrap());
    assert_eq!((at("very").bold, at("very").italic), (Some(true), None));
    assert_eq!((at("quite").italic, at("quite").bold), (Some(true), None));
    // Complex-script text (Arabic, Hebrew) takes the formatting too.
    assert_eq!((at("very").bold_cs, at("quite").italic_cs), (Some(true), Some(true)));
    assert_eq!(at("two").bold, Some(true));
    assert_eq!(at("words").bold, Some(true));
    for plain in ["say", " nice", ", ", " good", ". ok"] {
        assert_eq!((at(plain).bold, at(plain).italic), (None, None), "{plain:?}");
    }
    // Enter finishes the marked word too, and the next paragraph is not bold.
    let p = s.doc.para_at(&Pos::body(1, 0)).unwrap();
    assert_eq!((p.props_of_char(0).bold, p.mark.bold), (Some(true), None));
    let p = s.doc.para_at(&Pos::body(2, 0)).unwrap();
    assert_eq!((p.props_of_char(0).bold, p.mark.bold), (None, None));
    // With Track Changes on, the paragraph marks carry the same revisions as for unmarked text.
    let marks = |keys: &str| {
        let mut s = self::s();
        run(&mut s, "tools.autocorrect", json!({"boldItalic": true}));
        run(&mut s, "review.trackChanges", json!({"value": true}));
        typ(&mut s, keys);
        s.doc.body.iter().filter_map(|b| b.as_para()).map(|p| (p.mark.ins.is_some(), p.mark.del.is_some())).collect::<Vec<_>>()
    };
    assert_eq!(marks("x *end*\n"), marks("x end\n"));
    // After a no-break space too.
    let mut s = self::s();
    run(&mut s, "tools.autocorrect", json!({"boldItalic": true}));
    typ(&mut s, "a\u{a0}*word* ");
    assert_eq!(s.doc.plain_text(StoryRef::Body), "a\u{a0}word ");
    // A red word between plain marks: the next paragraph takes the closing mark's formatting.
    let mut s = self::s();
    run(&mut s, "tools.autocorrect", json!({"boldItalic": true}));
    typ(&mut s, "*");
    run(&mut s, "format.color", json!({"color": "FF0000"}));
    typ(&mut s, "red");
    run(&mut s, "format.color", json!({"color": "auto"}));
    typ(&mut s, "*\nnext");
    let red = s.doc.para_at(&Pos::body(0, 0)).unwrap().props_of_char(0).color;
    let p = s.doc.para_at(&Pos::body(1, 0)).unwrap();
    assert!(red.is_some());
    assert_ne!(p.mark.color, red);
    assert_ne!(p.props_of_char(0).color, red);
    // A heading's next style starts afresh after Enter, as without the marks.
    let mut s = self::s();
    run(&mut s, "tools.autocorrect", json!({"boldItalic": true}));
    run(&mut s, "para.heading1", json!({}));
    run(&mut s, "format.bold", json!({}));
    typ(&mut s, "*title*\n");
    let p = s.doc.para_at(&Pos::body(1, 0)).unwrap();
    assert_eq!((p.props.style.as_deref(), p.mark.bold), (Some("Normal"), None));
    // Marks typed before Track Changes was turned on stay: removing them would not be tracked.
    let mut s = self::s();
    run(&mut s, "tools.autocorrect", json!({"boldItalic": true}));
    typ(&mut s, "x *end*");
    run(&mut s, "review.trackChanges", json!({"value": true}));
    typ(&mut s, " ");
    assert_eq!(s.doc.plain_text(StoryRef::Body), "X *end* ");
    // Marks the same author typed with Track Changes on are converted.
    let mut s = self::s();
    run(&mut s, "tools.autocorrect", json!({"boldItalic": true}));
    run(&mut s, "review.trackChanges", json!({"value": true}));
    typ(&mut s, "x *end* ");
    assert_eq!(s.doc.plain_text(StoryRef::Body), "X end ");
    // Marks another author inserted stay.
    let mut s = self::s();
    run(&mut s, "tools.autocorrect", json!({"boldItalic": true}));
    run(&mut s, "review.trackChanges", json!({"value": true}));
    run(&mut s, "file.setAuthor", json!({"name": "Ana"}));
    typ(&mut s, "x *end*");
    run(&mut s, "file.setAuthor", json!({"name": "Ben"}));
    typ(&mut s, " ");
    assert_eq!(s.doc.plain_text(StoryRef::Body), "X *end* ");
    // Undo right after brings the marks back.
    let mut s = self::s();
    run(&mut s, "tools.autocorrect", json!({"boldItalic": true}));
    typ(&mut s, "a *b* ");
    assert_eq!(s.doc.plain_text(StoryRef::Body), "A b ");
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(s.doc.plain_text(StoryRef::Body), "A *b* ");
    // After Enter, Undo brings the marks back and the Enter stays.
    let mut s = self::s();
    run(&mut s, "tools.autocorrect", json!({"boldItalic": true}));
    typ(&mut s, "a *b*\n");
    assert_eq!(s.doc.plain_text(StoryRef::Body), "A b\n");
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(s.doc.plain_text(StoryRef::Body), "A *b*\n");
}

#[test]
fn autocorrect_goes_on_after_bold_and_italic_marks() {
    // After a space or punctuation, the word without its marks is replaced and capitalized.
    let mut s = s();
    run(&mut s, "tools.autocorrect", json!({"boldItalic": true, "add": {"from": "wc", "to": "WordCraft"}}));
    typ(&mut s, "*teh* end. _wc_.");
    assert_eq!(s.doc.plain_text(StoryRef::Body), "The end. WordCraft.");
    let p = s.doc.para_at(&Pos::body(0, 0)).unwrap();
    let at = |t: &str| p.props_of_char(p.text.find(t).unwrap()).clone();
    assert_eq!((at("The").bold, at("WordCraft").italic, at(" end").bold), (Some(true), Some(true), None));
    // One Undo brings the marks back as typed.
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(s.doc.plain_text(StoryRef::Body), "The end. _wc_.");
    // A marked web address becomes a link, and the text typed after it is neither bold nor italic.
    let mut s = self::s();
    run(&mut s, "tools.autocorrect", json!({"boldItalic": true}));
    typ(&mut s, "see *www.example.org* and _www.example.com_ next ");
    let p = s.doc.para_at(&Pos::body(0, 0)).unwrap();
    let at = |t: &str| p.props_of_char(p.text.find(t).unwrap()).clone();
    assert_eq!((at("www.example.org").bold, at("www.example.org").link.is_some()), (Some(true), true));
    assert_eq!((at("www.example.com").italic, at("www.example.com").link.is_some()), (Some(true), true));
    for plain in [" and", " next"] {
        assert_eq!((at(plain).bold, at(plain).italic, at(plain).link.is_some()), (None, None, false), "{plain:?}");
    }
    // After Enter too; a longer replacement moves the Enter with it, and Undo keeps the Enter.
    let mut s = self::s();
    run(&mut s, "tools.autocorrect", json!({"boldItalic": true, "add": {"from": "wc", "to": "WordCraft"}}));
    typ(&mut s, "*teh*\nok. *wc*\nnext");
    assert_eq!(s.doc.plain_text(StoryRef::Body), "The\nOk. WordCraft\nnext");
    let p = s.doc.para_at(&Pos::body(1, 0)).unwrap();
    assert_eq!(p.props_of_char(p.text.find("WordCraft").unwrap()).bold, Some(true));
    let mut s = self::s();
    run(&mut s, "tools.autocorrect", json!({"boldItalic": true}));
    typ(&mut s, "*teh*\n");
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(s.doc.plain_text(StoryRef::Body), "*teh*\n");
    // In a span of several words, the first word is replaced and capitalized as it is typed.
    let mut s = self::s();
    run(&mut s, "tools.autocorrect", json!({"boldItalic": true}));
    typ(&mut s, "*teh end* and _teh rest_ ");
    assert_eq!(s.doc.plain_text(StoryRef::Body), "The end and the rest ");
    let p = s.doc.para_at(&Pos::body(0, 0)).unwrap();
    let at = |t: &str| p.props_of_char(p.text.find(t).unwrap()).clone();
    assert_eq!((at("The end").bold, at("the rest").italic, at(" and").bold), (Some(true), Some(true), None));
}

#[test]
fn bold_and_italic_marks_stay_as_typed_unless_they_enclose_words() {
    let mut s = s();
    run(&mut s, "tools.autocorrect", json!({"boldItalic": true, "capSentences": false}));
    let keys = "2*3* file_name_ * x* *x * ** __ *?!* _-_ ";
    typ(&mut s, keys);
    assert_eq!(s.doc.plain_text(StoryRef::Body), keys);
    // Marks stay while formatting is limited to the document's styles.
    let mut s = self::s();
    run(&mut s, "tools.autocorrect", json!({"boldItalic": true}));
    run(&mut s, "review.restrict", json!({"mode": "none", "formatting": true}));
    typ(&mut s, "*word* ");
    assert_eq!(s.doc.plain_text(StoryRef::Body), "*word* ");
    // Marks around an object (here a check box content control) stay.
    let mut s = self::s();
    run(&mut s, "tools.autocorrect", json!({"boldItalic": true}));
    typ(&mut s, "x *");
    run(&mut s, "developer.checkBox", json!({}));
    run(&mut s, "caret.docEnd", json!({}));
    typ(&mut s, "* ");
    let p = s.doc.para_at(&Pos::body(0, 0)).unwrap();
    assert_eq!(p.text.matches('*').count(), 2, "{:?}", p.text);
    // Off by default.
    let s = typed("*bold* _italic_ ");
    assert_eq!(s.doc.plain_text(StoryRef::Body), "*bold* _italic_ ");
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

/// #2036: a dash AutoFormat makes right after a symbol is text, not the symbol's font.
#[test]
fn a_dash_after_a_symbol_is_not_in_the_symbol_font() {
    let mut s = s();
    typ(&mut s, "e");
    run(&mut s, "insert.symbol", json!({"char": "\u{F0FC}", "font": "Wingdings"}));
    typ(&mut s, "--f ");
    let p = s.doc.para_at(&Pos::body(0, 0)).unwrap();
    let dash = p.text.find('\u{2014}').expect("em dash");
    assert_eq!(p.props_of_char(dash).sym_font, None);
}

/// #2036: the en dashes of "x -- y" and "x - y" after a symbol (and its symbol-font space) are
/// text, not the symbol's font.
#[test]
fn en_dashes_after_a_symbol_are_not_in_the_symbol_font() {
    for keys in ["-- f ", "- f "] {
        let mut s = s();
        typ(&mut s, "e");
        run(&mut s, "insert.symbol", json!({"char": "\u{F0FC} ", "font": "Wingdings"}));
        typ(&mut s, keys);
        let p = s.doc.para_at(&Pos::body(0, 0)).unwrap();
        let dash = p.text.find('\u{2013}').unwrap_or_else(|| panic!("en dash for {keys:?}: {:?}", p.text));
        assert_eq!(p.props_of_char(dash).sym_font, None, "{keys:?}");
    }
}

/// #2036: an AutoCorrect replacement of a word that starts with a symbol-font character is text.
#[test]
fn an_autocorrect_replacement_of_a_symbol_word_is_not_in_the_symbol_font() {
    let mut s = s();
    run(&mut s, "insert.symbol", json!({"char": "(", "font": "Symbol"}));
    typ(&mut s, "c) ");
    let p = s.doc.para_at(&Pos::body(0, 0)).unwrap();
    let c = p.text.find('\u{A9}').unwrap_or_else(|| panic!("(c) → ©: {:?}", p.text));
    assert_eq!(p.props_of_char(c).sym_font, None);
}

/// #2036: a letter AutoCorrect capitalises after a symbol-font character is text, not the symbol's font.
#[test]
fn a_capitalised_word_after_a_symbol_is_not_in_the_symbol_font() {
    let mut s = s();
    run(&mut s, "insert.symbol", json!({"char": "m", "font": "Wingdings"}));
    typ(&mut s, "onday ");
    let p = s.doc.para_at(&Pos::body(0, 0)).unwrap();
    let m = p.text.find('M').unwrap_or_else(|| panic!("monday → Monday: {:?}", p.text));
    assert_eq!(p.props_of_char(m).sym_font, None);
}

/// Whether every character of the body paragraph `i` is marked deleted.
fn all_deleted(s: &Session, i: usize, text: &str) -> bool {
    s.doc.para_at(&Pos::body(i, 0)).is_some_and(|p| p.text == text && p.run_ranges().all(|(_, c)| c.del.is_some()))
}

#[test]
fn a_tracked_border_line_is_a_deletion_and_a_formatting_change() {
    let bottom = |s: &Session, i| s.doc.para_at(&Pos::body(i, 0)).and_then(|p| p.props.borders).and_then(|b| b.bottom);
    let tracked = |s: &Session, i| s.doc.para_at(&Pos::body(i, 0)).is_some_and(|p| p.props.fmt_change.is_some());
    // Characters that were there before Track Changes was turned on are marked deleted.
    let mut s = typed("Above\n---");
    run(&mut s, "review.trackChanges", json!({"value": true}));
    typ(&mut s, "\n");
    assert!(bottom(&s, 0).is_some() && tracked(&s, 0));
    assert!(all_deleted(&s, 1, "---"), "{:?}", paras(&s));
    // Enter again after the deleted characters is a plain Enter.
    typ(&mut s, "\n");
    assert_eq!(texts(&s), ["above", "---", ""]);
    assert_eq!(s.sel.focus, Pos::body(2, 0));
    run(&mut s, "edit.undo", json!({}));
    run(&mut s, "review.rejectAll", json!({}));
    assert_eq!(texts(&s), ["above", "---"]);
    assert!(bottom(&s, 0).is_none());
    run(&mut s, "edit.undo", json!({}));
    run(&mut s, "review.acceptAll", json!({}));
    assert_eq!(texts(&s), ["above", ""]);
    assert!(bottom(&s, 0).is_some());
    // At the top of the document the border goes on the characters' paragraph, and typing
    // continues in a new one.
    let mut s = typed("---");
    run(&mut s, "review.trackChanges", json!({"value": true}));
    typ(&mut s, "\nText");
    assert!(bottom(&s, 0).is_some() && tracked(&s, 0));
    assert!(all_deleted(&s, 0, "---"), "{:?}", paras(&s));
    assert_eq!(texts(&s)[1], "text");
    run(&mut s, "review.rejectAll", json!({}));
    assert_eq!(texts(&s), ["---"]);
    assert!(bottom(&s, 0).is_none());
    // Typed with Track Changes on, the characters go and the border is still a change.
    let mut s = typed("");
    run(&mut s, "review.trackChanges", json!({"value": true}));
    typ(&mut s, "---\n");
    assert_eq!(texts(&s), ["", ""]);
    assert!(bottom(&s, 0).is_some() && tracked(&s, 0));
    run(&mut s, "review.rejectAll", json!({}));
    assert!(bottom(&s, 0).is_none());
}

#[test]
fn a_tracked_table_line_keeps_its_characters_as_a_deletion() {
    let mut s = typed("+--+------+");
    run(&mut s, "review.trackChanges", json!({"value": true}));
    typ(&mut s, "\nA");
    let Some(wordcraft_doc::Block::Table(t)) = s.doc.body.first().map(|b| &**b) else { panic!("no table: {:?}", paras(&s)) };
    assert_eq!(t.rows[0].cells[0].blocks[0].as_para().map(|p| p.text.as_str()), Some("A"), "the caret is in the first cell");
    assert!(all_deleted(&s, 1, "+--+------+"), "{:?}", paras(&s));
    // Enter after the deleted characters makes no second table.
    run(&mut s, "caret.set", json!({"pos": {"block": 1, "off": 11}}));
    typ(&mut s, "\n");
    assert_eq!(s.doc.body.iter().filter(|b| b.as_para().is_none()).count(), 1, "{:?}", paras(&s));
    run(&mut s, "edit.undo", json!({}));
    run(&mut s, "review.rejectAll", json!({}));
    assert!(s.doc.body.get(1).and_then(|b| b.as_para()).is_some_and(|p| p.text == "+--+------+"));
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

/// The first paragraph's (left, first-line) indents as set on the paragraph itself.
fn indents(s: &Session) -> (Option<f32>, Option<f32>) {
    let p = s.doc.para_at(&Pos::body(0, 0)).unwrap();
    (p.props.indent_left, p.props.indent_first)
}

#[test]
fn tab_and_backspace_at_a_paragraph_start_set_its_indents() {
    let mut s = s();
    run(&mut s, "tools.autocorrect", json!({"tabIndent": true}));
    typ(&mut s, "Some text");
    run(&mut s, "caret.docStart", json!({}));
    // The first Tab indents the first line, the next one the whole paragraph.
    typ(&mut s, "\t");
    assert_eq!(indents(&s), (None, Some(36.0)));
    typ(&mut s, "\t");
    assert_eq!(indents(&s), (Some(36.0), Some(36.0)));
    assert_eq!(s.doc.plain_text(StoryRef::Body), "Some text");
    // Backspace takes the first-line indent back, then the left indent, and stops there.
    typ(&mut s, "\u{8}");
    assert_eq!(indents(&s), (Some(36.0), Some(0.0)));
    typ(&mut s, "\u{8}\u{8}");
    assert_eq!(indents(&s), (Some(0.0), Some(0.0)));
    assert_eq!(s.doc.plain_text(StoryRef::Body), "Some text");
    // Each Tab is one undo step.
    typ(&mut s, "\t");
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(indents(&s), (Some(0.0), Some(0.0)));
    // A left indent in characters goes back one tab stop too, instead of the paragraphs joining:
    // six characters of 11 or 12 points are 66 or 72 points, two tab stops.
    let mut s = self::s();
    run(&mut s, "tools.autocorrect", json!({"tabIndent": true}));
    typ(&mut s, "One\nTwo");
    let second = Pos::body(1, 0);
    s.doc.para_mut(StoryRef::Body, &second.path).unwrap().props.indent_left_chars = Some(6.0);
    s.sel = crate::Selection::caret(second.clone());
    typ(&mut s, "\u{8}");
    assert_eq!(s.doc.plain_text(StoryRef::Body), "One\nTwo");
    let p = s.doc.para_at(&second).unwrap();
    assert_eq!((p.props.indent_left, p.props.indent_left_chars), (Some(36.0), Some(0.0)));
    // With formatting limited to the document's styles the option is off: Tab types a tab
    // character at the start of the first line and of a later one, and Backspace at the start
    // keeps the paragraph's left indent.
    let mut s = self::s();
    run(&mut s, "tools.autocorrect", json!({"tabIndent": true}));
    typ(&mut s, "One");
    run(&mut s, "text.lineBreak", json!({}));
    typ(&mut s, "Two");
    run(&mut s, "review.restrict", json!({"mode": "none", "formatting": true}));
    run(&mut s, "caret.home", json!({}));
    typ(&mut s, "\t");
    run(&mut s, "caret.docStart", json!({}));
    typ(&mut s, "\t");
    assert_eq!(s.doc.plain_text(StoryRef::Body), "\tOne\n\tTwo");
    assert_eq!(indents(&s), (None, None));
    s.doc.para_mut(StoryRef::Body, &Pos::body(0, 0).path).unwrap().props.indent_left = Some(36.0);
    typ(&mut s, "\u{8}\u{8}");
    assert_eq!(s.doc.plain_text(StoryRef::Body), "One\n\tTwo");
    assert_eq!(indents(&s), (Some(36.0), None));
    // An empty paragraph takes a tab character.
    let mut s = self::s();
    run(&mut s, "tools.autocorrect", json!({"tabIndent": true}));
    typ(&mut s, "\tText");
    assert_eq!(s.doc.plain_text(StoryRef::Body), "\tText");
    assert_eq!(indents(&s), (None, None));
    // At the start of a list item numbered by its style, Tab types a tab character, as with the
    // option off.
    let mut s = style_inherited_list(wordcraft_doc::numbering::ListKind::Numbered, false);
    run(&mut s, "tools.autocorrect", json!({"tabIndent": true}));
    run(&mut s, "caret.docStart", json!({}));
    typ(&mut s, "\t");
    assert_eq!(s.doc.plain_text(StoryRef::Body), "\tAlpha\nBeta");
    assert_eq!(indents(&s), (None, None));
    // With the option off (the default), Tab types a tab character.
    let mut s = self::s();
    typ(&mut s, "Some text");
    run(&mut s, "caret.docStart", json!({}));
    typ(&mut s, "\t");
    assert_eq!(s.doc.plain_text(StoryRef::Body), "\tSome text");
    assert_eq!(indents(&s), (None, None));
}

#[test]
fn tab_before_a_later_line_of_a_paragraph_indents_the_whole_paragraph() {
    let mut s = s();
    run(&mut s, "tools.autocorrect", json!({"tabIndent": true}));
    typ(&mut s, &"Words that run on past the end of one line. ".repeat(6));
    run(&mut s, "caret.docStart", json!({}));
    run(&mut s, "caret.down", json!({}));
    run(&mut s, "caret.home", json!({}));
    assert!(s.sel.focus.off > 0, "the caret is on the second line");
    typ(&mut s, "\t");
    assert_eq!(indents(&s), (Some(36.0), None));
    assert!(!s.doc.plain_text(StoryRef::Body).contains('\t'));
    // After a line break too; inside a line Tab still types a tab character.
    let mut s = self::s();
    run(&mut s, "tools.autocorrect", json!({"tabIndent": true}));
    typ(&mut s, "One");
    run(&mut s, "text.lineBreak", json!({}));
    typ(&mut s, "Two");
    run(&mut s, "caret.home", json!({}));
    typ(&mut s, "\t");
    assert_eq!(indents(&s), (Some(36.0), None));
    run(&mut s, "caret.right", json!({}));
    typ(&mut s, "\t");
    assert_eq!(s.doc.plain_text(StoryRef::Body), "One\nT\two");
    // Before a later line of a list item numbered by its style, Tab takes it down a level, as
    // for a list item numbered directly.
    let mut s = style_inherited_list(wordcraft_doc::numbering::ListKind::Numbered, false);
    run(&mut s, "tools.autocorrect", json!({"tabIndent": true}));
    run(&mut s, "caret.docStart", json!({}));
    run(&mut s, "caret.end", json!({}));
    run(&mut s, "text.lineBreak", json!({}));
    typ(&mut s, "More");
    run(&mut s, "caret.home", json!({}));
    typ(&mut s, "\t");
    assert!(!s.doc.plain_text(StoryRef::Body).contains('\t'));
    let p = s.doc.para_at(&Pos::body(0, 0)).unwrap();
    assert_eq!(p.props.numbering.map(|n| n.level), Some(1));
    assert_eq!(indents(&s), (None, None));
    // A list level past the last one, from a style or set directly, goes to the last level.
    let mut s = style_inherited_list(wordcraft_doc::numbering::ListKind::Numbered, false);
    let style = s.doc.styles.styles.iter_mut().find(|x| x.id == "ListBase").unwrap();
    style.para.numbering = style.para.numbering.map(|n| wordcraft_doc::props::NumRef { level: u8::MAX, ..n });
    run(&mut s, "tools.autocorrect", json!({"tabIndent": true}));
    run(&mut s, "caret.docStart", json!({}));
    run(&mut s, "caret.end", json!({}));
    run(&mut s, "text.lineBreak", json!({}));
    typ(&mut s, "More");
    run(&mut s, "caret.home", json!({}));
    typ(&mut s, "\t");
    assert_eq!(s.doc.para_at(&Pos::body(0, 0)).unwrap().props.numbering.map(|n| n.level), Some(8));
    let num = s.doc.para_at(&Pos::body(0, 0)).unwrap().props.numbering.unwrap().num;
    s.doc.para_mut(StoryRef::Body, &Pos::body(1, 0).path).unwrap().props.numbering = Some(wordcraft_doc::props::NumRef { num, level: u8::MAX });
    s.sel = crate::Selection::caret(Pos::body(1, 0));
    typ(&mut s, "\t");
    assert_eq!(s.doc.para_at(&Pos::body(1, 0)).unwrap().props.numbering.map(|n| n.level), Some(8));
    s.doc.para_mut(StoryRef::Body, &Pos::body(0, 0).path).unwrap().props.numbering = Some(wordcraft_doc::props::NumRef { num, level: u8::MAX });
    run(&mut s, "caret.docStart", json!({}));
    run(&mut s, "caret.down", json!({}));
    run(&mut s, "caret.home", json!({}));
    assert_eq!((s.sel.focus.path.clone(), s.sel.focus.off), (Pos::body(0, 0).path, 6), "the caret is before \"More\"");
    typ(&mut s, "\t");
    assert_eq!(s.doc.para_at(&Pos::body(0, 0)).unwrap().props.numbering.map(|n| n.level), Some(8));
    s.doc.para_mut(StoryRef::Body, &Pos::body(0, 0).path).unwrap().props.numbering = Some(wordcraft_doc::props::NumRef { num, level: u8::MAX });
    run(&mut s, "outline.demote", json!({}));
    assert_eq!(s.doc.para_at(&Pos::body(0, 0)).unwrap().props.numbering.map(|n| n.level), Some(8));
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

/// Body paragraphs as (text, paragraph style).
fn styled(s: &Session) -> Vec<(String, Option<String>)> {
    s.doc.body.iter().filter_map(|b| b.as_para()).map(|p| (p.text.clone(), p.props.style.clone())).collect()
}

fn with_heading_styles(keys: &str) -> Session {
    let mut s = s();
    run(&mut s, "tools.autocorrect", json!({"headingStyles": true}));
    typ(&mut s, keys);
    s
}

#[test]
fn two_enters_after_a_short_line_make_it_a_heading() {
    let h = |t: &str, st: Option<&str>| (t.to_string(), st.map(str::to_string));
    // No tab: Heading 1; one or two tabs: Heading 2 or 3, without the tabs. Typing goes on in
    // the paragraph after the heading.
    for (keys, style) in [("Introduction\n\nText", "Heading1"), ("\tOur methods\n\nText", "Heading2"), ("\t\tStep 2\n\nText", "Heading3")] {
        let s = with_heading_styles(keys);
        let text = keys.trim_start_matches('\t').split('\n').next().unwrap_or_default();
        assert_eq!(styled(&s), [h(text, Some(style)), h("Text", None)], "{keys:?}");
    }
    // Five words is the most; one Enter only ends the line.
    let s = with_heading_styles("One two three four five\n\n");
    assert_eq!(s.doc.body.first().and_then(|b| b.as_para()).and_then(|p| p.props.style.as_deref()), Some("Heading1"));
    assert_eq!(styled(&with_heading_styles("Introduction\nText")), [h("Introduction", None), h("Text", None)]);
    // Not a heading: six words, a closing punctuation mark, three tabs, a list item, a style.
    for keys in ["One two three four five six\n\n", "Dear Sir,\n\n", "Is it?\n\n", "\t\t\tDeep\n\n", "* Item\n\n"] {
        let s = with_heading_styles(keys);
        assert!(
            s.doc.body.iter().filter_map(|b| b.as_para()).all(|p| p.props.style.as_deref().is_none_or(|st| !st.starts_with("Heading"))),
            "{keys:?}: {:?}",
            styled(&s)
        );
    }
    let mut q = self::s();
    run(&mut q, "tools.autocorrect", json!({"headingStyles": true}));
    run(&mut q, "para.style", json!({"style": "Quote"}));
    typ(&mut q, "Quoted\n\n");
    assert_eq!(q.doc.body.first().and_then(|b| b.as_para()).and_then(|p| p.props.style.as_deref()), Some("Quote"));
    // A list item numbered by the Normal style is not body text either.
    let mut q = self::s();
    run(&mut q, "tools.autocorrect", json!({"headingStyles": true}));
    let num = q.doc.numbering.try_add_list(wordcraft_doc::ListKind::Bullet).unwrap();
    if let Some(normal) = q.doc.styles.get_mut("Normal") {
        normal.para.numbering = Some(wordcraft_doc::props::NumRef { num, level: 0 });
    }
    typ(&mut q, "Item\n\n");
    assert!(styled(&q).iter().all(|(_, st)| st.as_deref().is_none_or(|st| !st.starts_with("Heading"))), "{:?}", styled(&q));
    // Off by default.
    let s = typed("Introduction\n\n");
    assert_eq!(styled(&s), [h("Introduction", None), h("", None), h("", None)]);
}

#[test]
fn undo_right_after_a_heading_autoformat_keeps_the_enter() {
    let mut s = with_heading_styles("\tMethods\n\n");
    assert_eq!(styled(&s), [("Methods".to_string(), Some("Heading2".to_string())), (String::new(), None)]);
    assert_eq!(s.sel.focus, Pos::body(1, 0));
    run(&mut s, "edit.undo", json!({}));
    // The AutoFormat is undone on its own: the tab and Normal text are back, the Enter stays.
    assert_eq!(s.doc.plain_text(StoryRef::Body), "\tMethods\n\n");
    assert!(styled(&s).iter().all(|(_, st)| st.is_none()));
    assert_eq!(s.sel.focus, Pos::body(2, 0));
}

#[test]
fn a_tracked_heading_autoformat_can_be_rejected() {
    // With Track Changes on, the tab is marked deleted and the style change is a formatting
    // revision, so Reject All brings the line back as it was.
    let mut s = with_heading_styles("\tMethods\n");
    run(&mut s, "review.trackChanges", json!({"value": true}));
    typ(&mut s, "\n");
    let p = s.doc.para_at(&Pos::body(0, 0)).cloned().unwrap_or_default();
    assert_eq!((p.text.as_str(), p.props.style.as_deref()), ("\tMethods", Some("Heading2")));
    assert!(p.run_ranges().any(|(r, c)| r.start == 0 && c.del.is_some()), "the tab is marked deleted");
    assert!(p.props.fmt_change.is_some(), "the style change is tracked");
    run(&mut s, "review.rejectAll", json!({}));
    assert_eq!(styled(&s).first(), Some(&("\tMethods".to_string(), None)));
    // A tab this author inserted is removed outright, and the style change is still tracked.
    let mut s = with_heading_styles("Title\n");
    run(&mut s, "review.trackChanges", json!({"value": true}));
    run(&mut s, "caret.set", json!({"pos": {"block": 0, "off": 0}}));
    typ(&mut s, "\t");
    run(&mut s, "caret.set", json!({"pos": {"block": 1, "off": 0}}));
    typ(&mut s, "\n");
    let p = s.doc.para_at(&Pos::body(0, 0)).cloned().unwrap_or_default();
    assert_eq!((p.text.as_str(), p.props.style.as_deref()), ("Title", Some("Heading2")));
    assert!(p.props.fmt_change.is_some(), "the style change is tracked");
    run(&mut s, "review.rejectAll", json!({}));
    assert_eq!(styled(&s).first(), Some(&("Title".to_string(), None)));
    // A line holding a tracked deletion is left alone.
    let mut s = with_heading_styles("Old title\n");
    run(&mut s, "review.trackChanges", json!({"value": true}));
    run(&mut s, "caret.set", json!({"pos": {"block": 0, "off": 3}}));
    typ(&mut s, "\u{8}\u{8}\u{8}");
    run(&mut s, "caret.set", json!({"pos": {"block": 1, "off": 0}}));
    typ(&mut s, "\n");
    assert!(styled(&s).iter().all(|(_, st)| st.is_none()), "{:?}", styled(&s));
}

#[test]
fn a_locked_heading_style_is_not_applied_as_you_type() {
    let mut s = s();
    run(&mut s, "tools.autocorrect", json!({"headingStyles": true}));
    run(&mut s, "review.restrict", json!({"mode": "none", "formatting": true, "lockedStyles": ["Heading1"]}));
    typ(&mut s, "Locked\n\n\tOpen\n\n");
    let st = styled(&s);
    assert_eq!(st.first(), Some(&("Locked".to_string(), None)), "{st:?}");
    assert!(st.contains(&("Open".to_string(), Some("Heading2".to_string()))), "{st:?}");
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
fn backspace_at_the_start_of_the_document_is_not_an_edit() {
    // #23: a key that changes nothing adds no undo step, marks no unsaved changes and
    // leaves the redo stack alone.
    let mut s = typed("alpha");
    run(&mut s, "caret.set", json!({"pos": {"block": 0, "off": 0}}));
    s.dirty = false;
    let depth = s.undo_depth();
    typ(&mut s, "\u{8}");
    assert_eq!(s.undo_depth(), depth, "no undo step");
    assert!(!s.dirty, "no unsaved changes");
    assert_eq!(s.doc.para_at(&Pos::body(0, 0)).unwrap().text, "alpha");
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
    // Paste of plain text from another program goes the same way.
    for (id, v) in [
        ("edit.pasteText", json!({"text": "A\n\nB"})),
        ("edit.pasteSpecial", json!({"as": "text", "text": "A\n\nB"})),
        ("edit.paste", json!({"text": "A\n\nB"})),
    ] {
        let mut s = s();
        run(&mut s, "para.bullets", json!({}));
        run(&mut s, id, v);
        assert_eq!(paras(&s), vec![para("A", Some(0)), para("", Some(0)), para("B", Some(0))], "{id}");
    }
    // Enter twice still ends the list.
    let mut s = s();
    run(&mut s, "para.bullets", json!({}));
    typ(&mut s, "A\n\nB");
    assert_eq!(paras(&s), vec![para("A", Some(0)), para("B", None)]);
}

/// Each body paragraph's text and paragraph formatting.
fn para_formats(s: &Session) -> Vec<(String, wordcraft_doc::props::ParaProps)> {
    s.doc.body.iter().filter_map(|b| b.as_para()).map(|p| (p.text.clone(), p.props.clone())).collect()
}

/// Plain text from another program pastes as Keep Text Only does, as in Word: every pasted line
/// becomes a paragraph formatted like the paragraph it lands in. Paste gave the lines after the
/// first default formatting, and the text after the caret went with the last line, so a
/// right-to-left paragraph came out left-to-right from its second line on.
#[test]
fn plain_text_paste_gives_every_line_the_formatting_of_its_paragraph() {
    let mut s = s();
    run(&mut s, "text.insert", json!({"text": "بداية نهاية", "raw": true}));
    run(&mut s, "para.rtl", json!({}));
    run(&mut s, "para.indents", json!({"left": 18, "right": 36}));
    run(&mut s, "para.spacing", json!({"after": 12}));
    let before = para_formats(&s);
    let [(_, props)] = &before[..] else { panic!("{before:?}") };
    assert_eq!(props.bidi, Some(true));
    let off = "بداية ".len();
    for id in ["edit.paste", "edit.pasteText"] {
        run(&mut s, "caret.set", json!({"pos": {"block": 0, "off": off}}));
        run(&mut s, id, json!({"text": "سطر أول\r\nسطر ثان\r\nسطر ثالث"}));
        let want: Vec<(String, _)> = ["بداية سطر أول", "سطر ثان", "سطر ثالثنهاية"].iter().map(|t| (t.to_string(), props.clone())).collect();
        assert_eq!(para_formats(&s), want, "{id}");
        // One undo step takes the whole paste out again.
        run(&mut s, "edit.undo", json!({}));
        assert_eq!(para_formats(&s), before, "{id}");
    }
}

/// Empty lines in pasted plain text come through as empty paragraphs, formatted like the rest,
/// and so does a line break at the end of the text. The text takes the formatting before the
/// caret.
#[test]
fn plain_text_paste_keeps_empty_lines_as_paragraphs() {
    let mut s = s();
    run(&mut s, "text.insert", json!({"text": "Lead ", "raw": true}));
    run(&mut s, "select.all", json!({}));
    run(&mut s, "format.bold", json!({}));
    run(&mut s, "caret.docEnd", json!({}));
    run(&mut s, "para.alignCenter", json!({}));
    run(&mut s, "para.spacing", json!({"before": 6, "after": 18}));
    let props = para_formats(&s)[0].1.clone();
    run(&mut s, "edit.paste", json!({"text": "Alpha one.\n\nAlpha two.\n\n\nAlpha three.\n"}));
    let got = para_formats(&s);
    let texts: Vec<&str> = got.iter().map(|(t, _)| t.as_str()).collect();
    assert_eq!(texts, ["Lead Alpha one.", "", "Alpha two.", "", "", "Alpha three.", ""]);
    assert!(got.iter().all(|(_, p)| *p == props), "{got:?}");
    assert_eq!(s.sel.focus, Pos::body(6, 0), "the caret ends after the pasted text");
    let bold = s.doc.body.iter().filter_map(|b| b.as_para()).all(|p| p.runs.iter().all(|r| r.props.bold == Some(true)));
    assert!(bold, "the pasted text takes the bold before the caret");
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

/// Tab in a table typed in right-to-left text (#2634): like Word, a table inserted in a
/// right-to-left paragraph is a right-to-left table, so its first cell stands at the right and
/// Tab moves to the cell on the left (the next one in reading order); Shift+Tab comes back. A
/// table inserted in a left-to-right paragraph is unchanged: Tab moves right.
#[test]
fn tab_in_a_table_inserted_in_right_to_left_text_moves_left() {
    // The caret's cell column and the x of that cell on the page.
    fn caret_cell_x(s: &mut Session) -> (usize, f32) {
        let (_, r, c) = s.sel.focus.path.cell().expect("caret in a cell");
        let x = s
            .layout()
            .pages
            .iter()
            .flat_map(|p| p.items.iter())
            .find_map(|it| match it {
                wordcraft_layout::Placed::Cell { rect, row, cell, covered: false, .. } if *row == r && *cell == c => Some(rect.x),
                _ => None,
            })
            .expect("cell laid out");
        (c, x)
    }
    for (keys, rtl) in [("مرحبا\n", true), ("Hello\n", false)] {
        let mut s = s();
        typ(&mut s, keys);
        run(&mut s, "insert.table", json!({"rows": 1, "cols": 3}));
        let t = s.doc.body.iter().find_map(|b| b.as_table()).expect("table");
        assert_eq!(t.props.bidi_visual, rtl, "{keys:?}: the table takes the paragraph's direction");
        let (c0, x0) = caret_cell_x(&mut s);
        typ(&mut s, "\t");
        let (c1, x1) = caret_cell_x(&mut s);
        assert_eq!((c0, c1), (0, 1), "{keys:?}: Tab goes to the next cell in reading order");
        assert_eq!(x1 < x0, rtl, "{keys:?}: Tab moves {} ({x0} -> {x1})", if rtl { "left" } else { "right" });
        run(&mut s, "text.backTab", json!({}));
        assert_eq!(caret_cell_x(&mut s), (0, x0), "{keys:?}: Shift+Tab comes back");
    }
}

#[test]
fn ctrl_tab_inserts_a_tab_inside_a_table_cell() {
    let mut s = s();
    run(&mut s, "insert.table", json!({"rows": 1, "cols": 2}));
    typ(&mut s, "A");
    run(&mut s, "text.insertTab", json!({}));
    typ(&mut s, "B");
    let (_, r, c) = s.sel.focus.path.cell().expect("caret in a cell");
    assert_eq!((r, c), (0, 0), "Ctrl+Tab stays in the cell");
    assert_eq!(s.doc.para_at(&s.sel.focus).map(|p| p.text.as_str()), Some("A\tB"));
    let key = if cfg!(target_os = "macos") { "Alt+Tab" } else { "Mod+Tab" };
    assert_eq!(s.keymap.resolve(&s.registry, key).map(|c| c.id), Some("text.insertTab"));
}

/// A .docx of two paragraphs in two sections, as Word writes it: the first section's properties
/// in the first paragraph's mark (portrait), the last section's in the body (landscape, starting
/// as `kind`).
fn two_section_docx(kind: &str) -> Vec<u8> {
    const W: &str = "http://schemas.openxmlformats.org/wordprocessingml/2006/main";
    let files = [
        (
            "[Content_Types].xml",
            r#"<?xml version="1.0" encoding="UTF-8"?><Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="xml" ContentType="application/xml"/><Override PartName="/word/document.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml"/></Types>"#.to_string(),
        ),
        (
            "_rels/.rels",
            r#"<?xml version="1.0" encoding="UTF-8"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="word/document.xml"/></Relationships>"#.to_string(),
        ),
        (
            "word/document.xml",
            format!(
                r#"<w:document xmlns:w="{W}"><w:body><w:p><w:pPr><w:sectPr><w:pgSz w:w="12240" w:h="15840"/></w:sectPr></w:pPr><w:r><w:t>One.</w:t></w:r></w:p><w:p><w:r><w:t>Two.</w:t></w:r></w:p><w:sectPr><w:type w:val="{kind}"/><w:pgSz w:w="15840" w:h="12240" w:orient="landscape"/></w:sectPr></w:body></w:document>"#
            ),
        ),
    ];
    let mut zip = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    for (name, data) in files {
        zip.start_file(name, zip::write::SimpleFileOptions::default()).unwrap();
        std::io::Write::write_all(&mut zip, data.as_bytes()).unwrap();
    }
    zip.finish().unwrap().into_inner()
}

/// #2127: Backspace just after a section break, or Delete just before it, removes the break, as
/// in Word: the two paragraphs join, the text before the break takes the following section's
/// page setup, and Undo brings the break back. Imported breaks and ones made here alike.
#[test]
fn backspace_and_delete_remove_a_section_break() {
    let made_here = || {
        let mut s = typed("One.Two.");
        s.sel = crate::Selection::caret(Pos::body(0, 4));
        run(&mut s, "layout.break", json!({"kind": "nextPage"}));
        s
    };
    for source in ["nextPage", "continuous", "made here"] {
        for (key, caret) in [("text.backspace", Pos::body(1, 0)), ("text.delete", Pos::body(0, 4))] {
            let mut s = match source {
                "made here" => made_here(),
                kind => Session::new(crate::io::open_bytes("s.docx", &two_section_docx(kind)).unwrap()),
            };
            assert_eq!(s.doc.sections().len(), 2, "{source}");
            assert_eq!(s.doc.plain_text(StoryRef::Body), "One.\nTwo.", "{source}");
            let after = s.doc.sections().last().map(|(_, sp)| (**sp).clone()).unwrap();
            s.sel = crate::Selection::caret(caret);
            run(&mut s, key, json!({}));
            assert_eq!(s.doc.plain_text(StoryRef::Body), "One.Two.", "{source} {key}");
            let sections = s.doc.sections();
            assert_eq!(sections.len(), 1, "{source} {key}: the break is gone");
            assert_eq!(*sections[0].1, after, "{source} {key}: the following section's setup");
            run(&mut s, "edit.undo", json!({}));
            assert_eq!(s.doc.plain_text(StoryRef::Body), "One.\nTwo.", "{source} {key}: undo");
            assert_eq!(s.doc.sections().len(), 2, "{source} {key}: undo brings the break back");
        }
    }
}

/// A .docx laid out like the one attached to #2127: a continuous section break, with the
/// document's only header, on the mark of an empty paragraph after "One.", and the last
/// section (nextPage, no header of its own, so linked to the previous one) in the body.
/// `then` adds a paragraph after the break; without it the break is on the last paragraph and
/// the last section has none.
fn break_on_the_last_paragraph_docx(then: Option<&str>) -> Vec<u8> {
    const W: &str = "http://schemas.openxmlformats.org/wordprocessingml/2006/main";
    const R: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships";
    let then = then.map(|t| format!("<w:p><w:r><w:t>{t}</w:t></w:r></w:p>")).unwrap_or_default();
    let files = [
        (
            "[Content_Types].xml",
            r#"<?xml version="1.0" encoding="UTF-8"?><Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="xml" ContentType="application/xml"/><Override PartName="/word/document.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml"/><Override PartName="/word/header1.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.header+xml"/></Types>"#.to_string(),
        ),
        (
            "_rels/.rels",
            r#"<?xml version="1.0" encoding="UTF-8"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="word/document.xml"/></Relationships>"#.to_string(),
        ),
        (
            "word/_rels/document.xml.rels",
            r#"<?xml version="1.0" encoding="UTF-8"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId2" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/header" Target="header1.xml"/></Relationships>"#.to_string(),
        ),
        ("word/header1.xml", format!(r#"<w:hdr xmlns:w="{W}"><w:p><w:r><w:t>Lab report</w:t></w:r></w:p></w:hdr>"#)),
        (
            "word/document.xml",
            format!(
                r#"<w:document xmlns:w="{W}" xmlns:r="{R}"><w:body><w:p><w:r><w:t>One.</w:t></w:r></w:p><w:p><w:pPr><w:sectPr><w:headerReference w:type="default" r:id="rId2"/><w:type w:val="continuous"/><w:pgSz w:w="11920" w:h="16850"/><w:pgMar w:top="1320" w:right="1275" w:bottom="280" w:left="1417" w:header="480" w:footer="0" w:gutter="0"/></w:sectPr></w:pPr></w:p>{then}<w:sectPr><w:type w:val="nextPage"/><w:pgSz w:w="11920" w:h="16850"/><w:pgMar w:top="1320" w:right="1275" w:bottom="280" w:left="1417" w:header="480" w:footer="0" w:gutter="0"/></w:sectPr></w:body></w:document>"#
            ),
        ),
    ];
    let mut zip = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    for (name, data) in files {
        zip.start_file(name, zip::write::SimpleFileOptions::default()).unwrap();
        std::io::Write::write_all(&mut zip, data.as_bytes()).unwrap();
    }
    zip.finish().unwrap().into_inner()
}

/// #2127: Delete at the end of the last paragraph removes a section break on its mark too,
/// though there's no paragraph after it to join. The text takes the last section's setup, and
/// the header the last section showed by linking to the removed one stays. Removing the break
/// by joining two paragraphs keeps that header as well. Undo brings the break back.
#[test]
fn delete_removes_a_section_break_on_the_last_paragraph() {
    for then in [None, Some("Two.")] {
        let mut s = Session::new(crate::io::open_bytes("s.docx", &break_on_the_last_paragraph_docx(then)).unwrap());
        let sections = s.doc.sections();
        assert_eq!(sections.len(), 2, "{then:?}");
        let header = sections[0].1.headers.default;
        assert!(header.is_some() && sections[1].1.headers.default.is_none(), "{then:?}: the header is on the break's section");
        let mut after = sections[1].1.clone();
        after.headers.default = header;
        s.sel = crate::Selection::caret(Pos::body(1, 0));
        run(&mut s, "text.delete", json!({}));
        let text = if then.is_some() { "One.\nTwo." } else { "One.\n" };
        assert_eq!(s.doc.plain_text(StoryRef::Body), text, "{then:?}");
        let sections = s.doc.sections();
        assert_eq!(sections.len(), 1, "{then:?}: the break is gone");
        assert_eq!(*sections[0].1, after, "{then:?}: the last section's setup, with the header");
        run(&mut s, "edit.undo", json!({}));
        let sections = s.doc.sections();
        assert_eq!(sections.len(), 2, "{then:?}: undo brings the break back");
        assert_eq!((sections[0].1.headers.default, sections[1].1.headers.default), (header, None), "{then:?}: undo");
    }
}

fn first_letter_list(s: &Session) -> Vec<String> {
    s.prefs.autocorrect.exceptions.clone()
}

/// Undoing an automatic sentence capital adds the word before it to the First Letter exceptions
/// (Word: "Automatically add words to list", #1857); typing the line again isn't capitalised.
#[test]
fn undoing_a_sentence_capital_adds_a_first_letter_exception() {
    let mut s = typed("hello there. world ");
    assert_eq!(s.doc.plain_text(StoryRef::Body), "Hello there. World ");
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(s.doc.plain_text(StoryRef::Body), "Hello there. world ");
    assert_eq!(first_letter_list(&s), ["there."]);
    run(&mut s, "text.newParagraph", json!({}));
    typ(&mut s, "x there. world ");
    assert_eq!(s.doc.plain_text(StoryRef::Body), "Hello there. world \nX there. world ");
}

#[test]
fn backspacing_a_sentence_capital_and_retyping_adds_a_first_letter_exception() {
    let mut s = typed("hello there. world ");
    typ(&mut s, "\u{8}\u{8}\u{8}\u{8}\u{8}\u{8}world ");
    assert_eq!(s.doc.plain_text(StoryRef::Body), "Hello there. world ");
    assert_eq!(first_letter_list(&s), ["there."]);
}

#[test]
fn automatic_exceptions_can_be_turned_off() {
    let mut s = typed("");
    run(&mut s, "tools.autocorrect", json!({"autoAddFirstLetter": false}));
    typ(&mut s, "hello there. world ");
    run(&mut s, "edit.undo", json!({}));
    assert!(first_letter_list(&s).is_empty());
    let mut s = typed("");
    run(&mut s, "tools.autocorrect", json!({"autoAddFirstLetter": false}));
    typ(&mut s, "hello there. world \u{8}\u{8}\u{8}\u{8}\u{8}\u{8}world ");
    assert_eq!(s.doc.plain_text(StoryRef::Body), "Hello there. World ");
    assert!(first_letter_list(&s).is_empty());
}

#[test]
fn undoing_a_replacement_adds_no_exception() {
    let mut s = typed("so teh ");
    assert_eq!(s.doc.plain_text(StoryRef::Body), "So the ");
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(s.doc.plain_text(StoryRef::Body), "So teh ");
    assert!(first_letter_list(&s).is_empty());
    assert!(s.prefs.autocorrect.entries.is_empty() && s.prefs.autocorrect.removed.is_empty());
    typ(&mut s, "and teh ");
    assert_eq!(s.doc.plain_text(StoryRef::Body), "So teh and the ");
}

/// Checked in Word for Mac: the same rule as First Letter.
#[test]
fn undoing_an_initial_caps_fix_adds_an_initial_caps_exception() {
    let mut s = typed("so TWo ");
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(s.prefs.autocorrect.initial_caps_exceptions, ["TWo"]);
    typ(&mut s, "and TWo ");
    assert_eq!(s.doc.plain_text(StoryRef::Body), "So TWo and TWo ");
}

#[test]
fn undoing_other_edits_after_a_capital_adds_no_exception() {
    // Typing more, then one Undo, and a second Undo that reverses the capital's step later.
    let mut s = typed("hello there. world more text");
    run(&mut s, "edit.undo", json!({}));
    assert!(first_letter_list(&s).is_empty());
    run(&mut s, "edit.undo", json!({}));
    assert!(first_letter_list(&s).is_empty(), "only the Undo that reverses the capital itself counts");
}

#[test]
fn a_full_undo_history_does_not_match_later_undos() {
    let mut s = typed("");
    for _ in 0..300 {
        typ(&mut s, "x\n");
    }
    typ(&mut s, "hello there. world more text\nand more");
    run(&mut s, "edit.undo", json!({}));
    assert!(first_letter_list(&s).is_empty());
}

#[test]
fn a_new_document_forgets_the_capital() {
    let mut s = typed("hello there. world ");
    run(&mut s, "file.new", json!({}));
    typ(&mut s, "abc\ndef\nghi\njkl");
    for _ in 0..8 {
        let _ = s.run("edit.undo", &json!({}));
    }
    assert!(first_letter_list(&s).is_empty());
}

#[test]
fn an_overlong_word_is_not_added_as_an_exception() {
    let mut s = typed(&format!("hi {}. world ", "a".repeat(5000)));
    run(&mut s, "edit.undo", json!({}));
    assert!(first_letter_list(&s).is_empty());
}

/// Each body paragraph's laid-out number (`None`: none) and its style.
fn numbers(s: &mut Session) -> Vec<(Option<String>, String)> {
    let l = s.layout();
    let labels: Vec<Option<String>> = l
        .pages
        .iter()
        .flat_map(|p| &p.items)
        .filter_map(|i| {
            if let wordcraft_layout::Placed::Lines { para, l0: 0, .. } = i { Some(para.label.as_ref().map(|lb| lb.text.clone())) } else { None }
        })
        .collect();
    let styles = s.doc.body.iter().filter_map(|b| b.as_para()).map(|p| p.props.style.clone().unwrap_or_else(|| "Normal".into()));
    labels.into_iter().zip(styles).collect()
}

fn num(label: Option<&str>, style: &str) -> (Option<String>, String) {
    (label.map(str::to_string), style.to_string())
}

/// #1793 (Word for Mac, 2026-10-15): after the "1 / 1.1 / 1.1.1" heading list, a new Heading 2
/// typed after "Three" is numbered 2.1 on its own; the body text after it stays unnumbered.
#[test]
fn a_new_heading_after_a_heading_linked_list_is_numbered() {
    let mut s = s();
    typ(&mut s, "One");
    run(&mut s, "para.heading1", json!({}));
    typ(&mut s, "\nTwo");
    run(&mut s, "para.heading2", json!({}));
    typ(&mut s, "\nThree");
    run(&mut s, "para.heading1", json!({}));
    // The caret in "Three" (a Heading 1, the list's first level).
    run(&mut s, "list.multilevelPreset", json!({"preset": "headings"}));
    run(&mut s, "caret.docEnd", json!({}));
    typ(&mut s, "\n");
    run(&mut s, "para.heading2", json!({}));
    typ(&mut s, "Four\nBody");
    assert_eq!(
        numbers(&mut s),
        [num(Some("1"), "Heading1"), num(Some("1.1"), "Heading2"), num(Some("2"), "Heading1"), num(Some("2.1"), "Heading2"), num(None, "Normal")]
    );
}

/// #1793: plain Numbering on a Heading 1 numbers it "1." and keeps Heading 1 (Word: "1. One").
#[test]
fn numbering_a_heading_keeps_the_heading_style() {
    let mut s = s();
    typ(&mut s, "One");
    run(&mut s, "para.heading1", json!({}));
    run(&mut s, "para.numbering", json!({}));
    assert_eq!(numbers(&mut s), [num(Some("1."), "Heading1")]);
}

/// #1793: applying a heading style that carries list numbering numbers the paragraph, even one
/// whose numbering was switched off before.
#[test]
fn applying_a_numbered_heading_style_numbers_the_paragraph() {
    let mut s = s();
    typ(&mut s, "One");
    run(&mut s, "para.heading1", json!({}));
    run(&mut s, "list.multilevelPreset", json!({"preset": "headings"}));
    typ(&mut s, "\nTwo");
    run(&mut s, "para.numbering", json!({}));
    run(&mut s, "para.numbering", json!({"off": true}));
    assert_eq!(numbers(&mut s)[1].0, None);
    run(&mut s, "para.heading2", json!({}));
    assert_eq!(numbers(&mut s), [num(Some("1"), "Heading1"), num(Some("1.1"), "Heading2")]);
}

/// #1793: "1. " typed at the start of a heading numbers it and keeps the heading style; only
/// body text becomes List Paragraph.
#[test]
fn typing_1_dot_in_a_heading_keeps_the_heading_style() {
    let mut s = s();
    run(&mut s, "para.heading1", json!({}));
    typ(&mut s, "1. One\n");
    run(&mut s, "para.normal", json!({}));
    typ(&mut s, "1. Body");
    assert_eq!(numbers(&mut s), [num(Some("1."), "Heading1"), num(Some("1."), "ListParagraph")]);
    for (style, keys) in [("Heading2", "a) Part"), ("Subtitle", "1. Item")] {
        let mut s = self::s();
        run(&mut s, "para.style", json!({"style": style}));
        typ(&mut s, keys);
        let got = numbers(&mut s);
        assert_eq!(got.len(), 1);
        assert!(got[0].0.is_some(), "{style}: a list");
        assert_eq!(got[0].1, style);
    }
}

// Backspace and Delete on selected tables, rows and columns (Word for Mac, #1844).

/// "before", a 3×3 table with cells `a1`…`c3` (column letter, row number), "after"; the caret
/// in the cell at row `r`, column `c`.
fn table_between(r: u32, c: u32) -> Session {
    let mut s = typed("before\n");
    run(&mut s, "insert.table", json!({"rows": 3, "cols": 3}));
    typ(&mut s, "a1\tb1\tc1\ta2\tb2\tc2\ta3\tb3\tc3");
    run(&mut s, "caret.docEnd", json!({}));
    typ(&mut s, "after");
    let shape: Vec<&str> = s.doc.body.iter().map(|b| if b.as_table().is_some() { "table" } else { "para" }).collect();
    assert_eq!(shape, ["para", "table", "para"], "setup");
    run(&mut s, "caret.set", json!({"pos": {"story": "body", "path": [1, r, c, 0], "off": 0}}));
    s
}

fn tables(s: &Session) -> usize {
    s.doc.body.iter().filter(|b| b.as_table().is_some()).count()
}

/// The first top-level table's cells, row by row (tracked deletions included).
fn table_cells(s: &Session) -> Vec<Vec<String>> {
    s.doc
        .body
        .iter()
        .find_map(|b| b.as_table())
        .map(|t| {
            t.rows
                .iter()
                .map(|r| r.cells.iter().map(|c| c.blocks.iter().filter_map(|b| b.as_para()).map(|p| p.plain_text()).collect()).collect())
                .collect()
        })
        .unwrap_or_default()
}

#[test]
fn backspace_with_the_whole_table_selected_removes_the_table() {
    let mut s = table_between(1, 1);
    run(&mut s, "table.selectTable", json!({}));
    run(&mut s, "text.backspace", json!({}));
    assert_eq!(tables(&s), 0, "the table stays: {:?}", table_cells(&s));
    assert_eq!(texts(&s), ["before", "after"]);
    // The caret is in the paragraph that followed the table.
    assert_eq!(s.sel.focus, Pos { story: StoryRef::Body, path: wordcraft_doc::Path::top(1), off: 0 });
    assert!(s.sel.is_collapsed());
    // One undo brings it back.
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(tables(&s), 1);
    assert_eq!(table_cells(&s), [["a1", "b1", "c1"], ["a2", "b2", "c2"], ["a3", "b3", "c3"]]);
}

#[test]
fn delete_with_the_whole_table_selected_clears_the_cells_and_keeps_the_table() {
    let mut s = table_between(0, 0);
    run(&mut s, "table.selectTable", json!({}));
    run(&mut s, "text.delete", json!({}));
    assert_eq!(tables(&s), 1);
    assert_eq!(table_cells(&s), [["", "", ""], ["", "", ""], ["", "", ""]]);
    assert_eq!(texts(&s), ["before", "after"]);
}

#[test]
fn delete_or_backspace_with_the_table_and_the_paragraph_after_selected_removes_the_table() {
    for key in ["text.delete", "text.backspace"] {
        // From the first cell to the end of the document: Word's rule widens it to whole rows.
        let mut s = table_between(0, 0);
        run(&mut s, "caret.docEnd", json!({"extend": true}));
        run(&mut s, key, json!({}));
        assert_eq!(tables(&s), 0, "{key}: {:?}", table_cells(&s));
        assert_eq!(texts(&s), ["before", ""], "{key}");
    }
}

#[test]
fn delete_or_backspace_with_the_paragraph_before_and_the_table_selected_removes_the_table() {
    for key in ["text.delete", "text.backspace"] {
        // From the start of "before" down into the last row: the rows are taken whole.
        let mut s = table_between(0, 0);
        run(&mut s, "caret.docStart", json!({}));
        for _ in 0..3 {
            run(&mut s, "caret.down", json!({"extend": true}));
        }
        assert_eq!(s.sel.focus.path.cell().map(|(_, r, _)| r), Some(2), "{key}: the selection reaches the last row");
        run(&mut s, key, json!({}));
        assert_eq!(tables(&s), 0, "{key}: {:?}", table_cells(&s));
        assert_eq!(texts(&s), ["after"], "{key}");
        assert_eq!(s.sel.focus, Pos { story: StoryRef::Body, path: wordcraft_doc::Path::top(0), off: 0 }, "{key}");
    }
}

#[test]
fn backspace_deletes_selected_rows_and_delete_clears_them() {
    let mut s = table_between(1, 0);
    run(&mut s, "table.selectRow", json!({}));
    run(&mut s, "text.backspace", json!({}));
    assert_eq!(table_cells(&s), [["a1", "b1", "c1"], ["a3", "b3", "c3"]]);

    let mut s = table_between(1, 0);
    run(&mut s, "table.selectRow", json!({}));
    run(&mut s, "text.delete", json!({}));
    assert_eq!(table_cells(&s), [["a1", "b1", "c1"], ["", "", ""], ["a3", "b3", "c3"]]);
}

#[test]
fn backspace_deletes_selected_columns_and_delete_clears_them() {
    let mut s = table_between(0, 1);
    run(&mut s, "table.selectColumn", json!({}));
    run(&mut s, "text.backspace", json!({}));
    assert_eq!(table_cells(&s), [["a1", "c1"], ["a2", "c2"], ["a3", "c3"]]);

    let mut s = table_between(0, 1);
    run(&mut s, "table.selectColumn", json!({}));
    run(&mut s, "text.delete", json!({}));
    assert_eq!(table_cells(&s), [["a1", "", "c1"], ["a2", "", "c2"], ["a3", "", "c3"]]);
}

/// The first top-level table's cells, row by row, one entry per block: a paragraph's text, or
/// `<table>` for a nested table.
fn cell_blocks(s: &Session) -> Vec<Vec<Vec<String>>> {
    let block = |b: &std::sync::Arc<wordcraft_doc::Block>| b.as_para().map_or_else(|| "<table>".to_string(), |p| p.plain_text());
    s.doc
        .body
        .iter()
        .find_map(|b| b.as_table())
        .map(|t| t.rows.iter().map(|r| r.cells.iter().map(|c| c.blocks.iter().map(block).collect()).collect()).collect())
        .unwrap_or_default()
}

#[test]
fn delete_on_whole_cells_leaves_one_empty_paragraph_in_each() {
    // a1 starts with a nested table and holds three paragraphs (centred, centred, right-aligned),
    // b2 holds a nested table after its text (#2802).
    let setup = || {
        let mut s = table_between(0, 0);
        run(&mut s, "caret.set", json!({"pos": {"story": "body", "path": [1, 0, 0, 0], "off": 2}}));
        run(&mut s, "para.alignCenter", json!({}));
        typ(&mut s, "\nX\nY");
        run(&mut s, "para.alignRight", json!({}));
        run(&mut s, "caret.set", json!({"pos": {"story": "body", "path": [1, 1, 1, 0], "off": 2}}));
        run(&mut s, "text.newParagraph", json!({}));
        run(&mut s, "insert.table", json!({"rows": 1, "cols": 2}));
        typ(&mut s, "n");
        let b2 = |s: &Session| s.doc.body.get(1).and_then(|b| b.as_table()).map(|t| t.rows[1].cells[1].blocks.clone()).unwrap_or_default();
        let nested = b2(&s).iter().find(|b| b.as_table().is_some()).map(|b| (**b).clone()).expect("setup: a table nested in b2");
        s.doc.insert_block(StoryRef::Body, &wordcraft_doc::Path(vec![1, 0, 0, 0]), nested).unwrap();
        s
    };
    let all: Vec<(usize, usize)> = (0..3).flat_map(|r| (0..3).map(move |c| (r, c))).collect();
    let cases: [(&str, (usize, usize), &str, Vec<(usize, usize)>); 5] = [
        ("table.selectRow", (0, 0), "text.delete", vec![(0, 0), (0, 1), (0, 2)]),
        ("table.selectColumn", (0, 0), "text.delete", vec![(0, 0), (1, 0), (2, 0)]),
        ("table.selectRow", (1, 1), "text.delete", vec![(1, 0), (1, 1), (1, 2)]),
        ("table.selectTable", (0, 0), "text.delete", all),
        // One cell of several: Backspace clears it too.
        ("table.selectCell", (0, 0), "text.backspace", vec![(0, 0)]),
    ];
    for (select, (r, c), key, cleared) in cases {
        let mut s = setup();
        let before = cell_blocks(&s);
        assert_eq!(before[0][0], ["<table>", "a1", "X", "Y"], "setup");
        assert!(before[1][1].len() > 2 && before[1][1].contains(&"<table>".to_string()), "setup: {:?}", before[1][1]);
        // a1's first paragraph is after the nested table.
        let first = usize::from((r, c) == (0, 0));
        run(&mut s, "caret.set", json!({"pos": {"story": "body", "path": [1, r, c, first], "off": 0}}));
        run(&mut s, select, json!({}));
        run(&mut s, key, json!({}));
        let mut want = before.clone();
        for &(r, c) in &cleared {
            want[r][c] = vec![String::new()];
        }
        assert_eq!(cell_blocks(&s), want, "{select} {key}");
        // The paragraph left keeps the cell's last paragraph's formatting (its mark ends the cell).
        if cleared.contains(&(0, 0)) {
            let p = s.doc.para(StoryRef::Body, &wordcraft_doc::Path(vec![1, 0, 0, 0])).map(|p| p.props.align);
            assert_eq!(p, Some(Some(wordcraft_doc::props::Align::Right)), "{select} {key}");
        }
        assert!(s.sel.is_collapsed(), "{select} {key}");
        assert_eq!(s.sel.focus.path.cell().map(|(_, r, c)| (r, c)), cleared.first().copied(), "{select} {key}");
        // One undo brings everything back.
        run(&mut s, "edit.undo", json!({}));
        assert_eq!(cell_blocks(&s), before, "{select} {key}: undo");
    }
}

#[test]
fn backspace_on_a_selected_nested_table_removes_only_it() {
    let mut s = table_between(1, 1);
    run(&mut s, "caret.set", json!({"pos": {"story": "body", "path": [1, 1, 1, 0], "off": 2}}));
    run(&mut s, "text.newParagraph", json!({}));
    run(&mut s, "insert.table", json!({"rows": 2, "cols": 2}));
    typ(&mut s, "x");
    let cell_blocks = |s: &Session| {
        s.doc
            .body
            .iter()
            .find_map(|b| b.as_table())
            .and_then(|t| t.rows.get(1)?.cells.get(1))
            .map(|c| c.blocks.iter().filter(|b| b.as_table().is_some()).count())
    };
    assert_eq!(cell_blocks(&s), Some(1), "setup: a table nested in b2");
    run(&mut s, "table.selectTable", json!({}));
    run(&mut s, "text.backspace", json!({}));
    assert_eq!(cell_blocks(&s), Some(0), "the nested table stays");
    assert_eq!(tables(&s), 1, "the outer table goes too");
    // The caret is in the cell's paragraph after where the nested table was.
    assert_eq!(s.sel.focus.path.cell().map(|(t, r, c)| (t.0, r, c)), Some((vec![1], 1, 1)));
    assert!(s.doc.para_at(&s.sel.focus).is_some());
}

#[test]
fn backspace_on_a_selected_table_that_is_the_only_block_leaves_an_empty_paragraph() {
    let mut s = s();
    run(&mut s, "insert.table", json!({"rows": 2, "cols": 2}));
    typ(&mut s, "x");
    s.doc.body.retain(|b| b.as_table().is_some());
    assert_eq!(s.doc.body.len(), 1, "setup: the table is the only block");
    run(&mut s, "table.selectTable", json!({}));
    run(&mut s, "text.backspace", json!({}));
    assert_eq!(tables(&s), 0);
    assert_eq!(texts(&s), [""]);
    assert!(s.doc.para_at(&s.sel.focus).is_some(), "the caret is in a paragraph: {:?}", s.sel.focus);
}

#[test]
fn tracked_backspace_on_a_selected_table_marks_its_text_deleted_like_delete_table() {
    let mut s = table_between(0, 0);
    run(&mut s, "review.trackChanges", json!({"value": true}));
    run(&mut s, "table.selectTable", json!({}));
    run(&mut s, "text.backspace", json!({}));
    // Like Table Layout › Delete Table with tracking on: the structure stays, the text is marked deleted.
    assert_eq!(tables(&s), 1);
    assert_eq!(table_cells(&s), [["a1", "b1", "c1"], ["a2", "b2", "c2"], ["a3", "b3", "c3"]]);
    assert_eq!(s.doc.final_text(StoryRef::Body).replace(['\t', '\n'], ""), "Beforeafter");
    run(&mut s, "review.rejectAll", json!({}));
    assert_eq!(s.doc.final_text(StoryRef::Body).replace(['\t', '\n'], ""), "Beforea1b1c1a2b2c2a3b3c3after");
    assert!(s.doc.revisions.is_empty());
}

/// One selected cell isn't a whole row or column, even in a one-row or one-column table:
/// Backspace clears it (Word's cell selection has no end-of-row mark).
#[test]
fn backspace_on_one_selected_cell_of_a_one_row_or_one_column_table_clears_it() {
    for (rows, cols, at) in [(1, 3, [0, 1]), (3, 1, [1, 0])] {
        let mut s = typed("before\n");
        run(&mut s, "insert.table", json!({"rows": rows, "cols": cols}));
        typ(&mut s, "x\ty\tz");
        run(&mut s, "caret.set", json!({"pos": {"story": "body", "path": [1, at[0], at[1], 0], "off": 0}}));
        run(&mut s, "table.selectCell", json!({}));
        run(&mut s, "text.backspace", json!({}));
        let want: Vec<Vec<String>> =
            if rows == 1 { vec![vec!["x".into(), "".into(), "z".into()]] } else { vec![vec!["x".into()], vec!["".into()], vec!["z".into()]] };
        assert_eq!(table_cells(&s), want, "{rows}×{cols}");
    }
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

/// #1923: Turkish and Azerbaijani case mapping (i↔İ, ı↔I) in AutoCorrect, checked against Word
/// with Turkish proofing; other languages keep the Unicode default.
#[test]
fn autocorrect_capitalisation_follows_turkic_case_mapping() {
    for tag in ["tr-TR", "az-AZ"] {
        let mut s = s();
        run(&mut s, "review.language", json!({"lang": tag, "default": true}));
        typ(&mut s, "istanbul güzel. ");
        assert_eq!(s.doc.plain_text(StoryRef::Body), "İstanbul güzel. ", "{tag}: i capitalises to İ");
        let mut s = self::s();
        run(&mut s, "review.language", json!({"lang": tag, "default": true}));
        typ(&mut s, "İNgilizce ");
        assert_eq!(s.doc.plain_text(StoryRef::Body), "İngilizce ", "{tag}: İ kept, N lowercased, no combining dot");
        let mut s = self::s();
        run(&mut s, "review.language", json!({"lang": tag, "default": true}));
        typ(&mut s, "ılık ırmak. ");
        assert_eq!(s.doc.plain_text(StoryRef::Body), "Ilık ırmak. ", "{tag}: ı capitalises to I");
        let mut s = self::s();
        run(&mut s, "review.language", json!({"lang": tag, "default": true}));
        typ(&mut s, "iSTANBUL ");
        assert_eq!(s.doc.plain_text(StoryRef::Body), "İstanbul ", "{tag}: cAPS LOCK fix");
        let mut s = self::s();
        run(&mut s, "review.language", json!({"lang": tag, "default": true}));
        typ(&mut s, "kızım. TIklamak ");
        assert_eq!(s.doc.plain_text(StoryRef::Body), "Kızım. Tıklamak ", "{tag}: TWo INitial CApitals lowercases I to ı");
    }
    // Other languages are unchanged; a stray İ still lowercases to a single "i".
    let mut s = s();
    run(&mut s, "review.language", json!({"lang": "en-US", "default": true}));
    typ(&mut s, "istanbul güzel. ");
    assert_eq!(s.doc.plain_text(StoryRef::Body), "Istanbul güzel. ");
    let mut s = self::s();
    run(&mut s, "review.language", json!({"lang": "en-US", "default": true}));
    typ(&mut s, "İNgilizce ");
    assert_eq!(s.doc.plain_text(StoryRef::Body), "İngilizce ", "no combining dot in any language");
}

/// #3479: Format > Change Case follows the same Turkic case mapping as AutoCorrect, using
/// each run's own language; other languages keep the Unicode default.
#[test]
fn change_case_follows_turkic_case_mapping() {
    for tag in ["tr-TR", "az-AZ"] {
        for (mode, input, want) in [
            ("upper", "istanbul", "İSTANBUL"),
            ("lower", "IŞIK", "ışık"),
            ("title", "istanbul güzel", "İstanbul Güzel"),
            ("sentence", "istanbul. izmir", "İstanbul. İzmir"),
            ("toggle", "iI", "İı"),
        ] {
            let mut s = s();
            run(&mut s, "review.language", json!({"lang": tag, "default": true}));
            run(&mut s, "text.insert", json!({"text": input, "raw": true}));
            run(&mut s, "select.all", json!({}));
            run(&mut s, "format.changeCase", json!({"mode": mode}));
            assert_eq!(s.doc.plain_text(StoryRef::Body), want, "{tag} {mode}");
        }
    }
    // Other languages keep the Unicode default.
    let mut s = self::s();
    run(&mut s, "text.insert", json!({"text": "istanbul", "raw": true}));
    run(&mut s, "select.all", json!({}));
    run(&mut s, "format.changeCase", json!({"mode": "title"}));
    assert_eq!(s.doc.plain_text(StoryRef::Body), "Istanbul", "en-US");
    // Mixed runs: each run's own language decides.
    let mut s = self::s();
    run(&mut s, "text.insert", json!({"text": "IŞIK IŞIK", "raw": true}));
    run(&mut s, "caret.set", json!({"pos": Pos::body(0, 0)}));
    run(&mut s, "caret.set", json!({"pos": Pos::body(0, 5), "extend": true}));
    run(&mut s, "review.language", json!({"lang": "tr-TR"}));
    run(&mut s, "caret.set", json!({"pos": Pos::body(0, 6)}));
    run(&mut s, "caret.set", json!({"pos": Pos::body(0, 11), "extend": true}));
    run(&mut s, "review.language", json!({"lang": "en-US"}));
    run(&mut s, "select.all", json!({}));
    run(&mut s, "format.changeCase", json!({"mode": "lower"}));
    assert_eq!(s.doc.plain_text(StoryRef::Body), "ışık işik", "mixed runs");
}

// Spelling as you type (#1552): the word being typed gets no red squiggle until the caret leaves
// it; F7 and the Spelling pane still report it. See docs/typing-parity.md. Paragraphs start with
// a correct word so AutoCorrect's sentence capital doesn't change the misspelled one.

/// The words body paragraph `i` has a spelling squiggle under, in order.
fn squiggles(s: &mut Session, i: usize) -> Vec<String> {
    let path = wordcraft_doc::Path::top(i);
    let text = s.doc.para(StoryRef::Body, &path).map(|p| p.text.clone()).unwrap_or_default();
    let l = s.layout();
    l.pages
        .iter()
        .flat_map(|pg| pg.items.iter())
        .filter_map(|it| match it {
            wordcraft_layout::Placed::Lines { para, path: p, l0: 0, .. } if *p == path => {
                Some(para.issues.iter().filter(|i| !i.2).filter_map(|i| text.get(i.0..i.1).map(str::to_string)).collect::<Vec<_>>())
            }
            _ => None,
        })
        .flatten()
        .collect()
}

/// The misspelled words `review.issues` (the Spelling pane) reports, in order.
fn reported(s: &mut Session) -> Vec<String> {
    let issues = s.run("review.issues", &json!({})).unwrap();
    issues.as_array().unwrap().iter().filter(|i| i["kind"] == "spelling").map(|i| i["text"].as_str().unwrap().to_string()).collect()
}

#[test]
fn the_word_being_typed_is_not_marked_until_it_ends() {
    let mut s = typed("The helo");
    assert_eq!(squiggles(&mut s, 0), Vec::<String>::new(), "no squiggle while the word is typed");
    // F7 and the Spelling pane still see it.
    assert_eq!(reported(&mut s), ["helo"]);
    let next = s.run("review.spelling", &json!({})).unwrap();
    assert_eq!(next["text"], "helo");
    // A space ends the word.
    let mut s = typed("The helo ");
    assert_eq!(squiggles(&mut s, 0), ["helo"]);
    typ(&mut s, "wrld");
    assert_eq!(squiggles(&mut s, 0), ["helo"], "the next word is being typed");
    assert_eq!(reported(&mut s), ["helo", "wrld"]);
    // So does punctuation.
    typ(&mut s, ".");
    assert_eq!(squiggles(&mut s, 0), ["helo", "wrld"]);
    // Enter and Tab too.
    let mut s = typed("The helo\n");
    assert_eq!(squiggles(&mut s, 0), ["helo"]);
    let mut s = typed("The helo\t");
    assert_eq!(squiggles(&mut s, 0), ["helo"]);
}

#[test]
fn moving_the_caret_out_of_the_typed_word_marks_it() {
    for key in ["caret.left", "caret.home", "caret.wordLeft", "caret.docStart"] {
        let mut s = typed("The helo");
        run(&mut s, key, json!({}));
        assert_eq!(squiggles(&mut s, 0), ["helo"], "{key}");
        // Coming back to the end of the word doesn't hide the squiggle again.
        run(&mut s, "caret.end", json!({}));
        assert_eq!(s.sel.focus, Pos::body(0, 8));
        assert_eq!(squiggles(&mut s, 0), ["helo"], "{key} then End");
    }
    // A selection ends it too.
    let mut s = typed("The helo");
    run(&mut s, "select.all", json!({}));
    assert_eq!(squiggles(&mut s, 0), ["helo"]);
}

#[test]
fn a_click_elsewhere_marks_the_typed_word_and_a_click_back_keeps_the_mark() {
    let mut s = typed("Fine\nThe helo");
    assert_eq!(squiggles(&mut s, 1), Vec::<String>::new());
    // The canvas sets the caret directly on a click.
    s.sel = crate::Selection::caret(Pos::body(0, 2));
    assert_eq!(squiggles(&mut s, 1), ["helo"]);
    s.sel = crate::Selection::caret(Pos::body(1, 8));
    assert_eq!(squiggles(&mut s, 1), ["helo"], "an existing misspelled word keeps its squiggle");
    // Typing into it makes it the word being typed again.
    typ(&mut s, "o");
    assert_eq!(s.doc.plain_text(StoryRef::Body), "Fine\nThe heloo");
    assert_eq!(squiggles(&mut s, 1), Vec::<String>::new());
    typ(&mut s, " ");
    assert_eq!(squiggles(&mut s, 1), ["heloo"]);
}

#[test]
fn backspace_into_a_finished_word_edits_it_again() {
    let mut s = typed("The helo ");
    assert_eq!(squiggles(&mut s, 0), ["helo"]);
    typ(&mut s, "\u{8}");
    assert_eq!(squiggles(&mut s, 0), Vec::<String>::new());
    typ(&mut s, "\u{8}\u{8}llo ");
    assert_eq!(s.doc.plain_text(StoryRef::Body), "The hello ");
    assert_eq!(squiggles(&mut s, 0), Vec::<String>::new());
}

#[test]
fn autocorrect_and_inserted_text_are_marked_as_usual() {
    // AutoCorrect fixes the word as the space ends it: nothing to mark.
    let mut s = typed("I typed teh ");
    assert_eq!(s.doc.plain_text(StoryRef::Body), "I typed the ");
    assert_eq!(squiggles(&mut s, 0), Vec::<String>::new());
    // Text inserted whole (a paste, an agent): only the word the caret ends in is being typed.
    let mut s = typed("");
    run(&mut s, "text.insert", json!({"text": "The helo wrld"}));
    assert_eq!(squiggles(&mut s, 0), ["helo"]);
    // Undo moves the caret: whatever is left is marked, as the Spelling pane reports it.
    let mut s = typed("The helo");
    run(&mut s, "caret.left", json!({}));
    typ(&mut s, "x");
    assert_eq!(s.doc.plain_text(StoryRef::Body), "The helxo");
    assert_eq!(squiggles(&mut s, 0), Vec::<String>::new());
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(s.doc.plain_text(StoryRef::Body), "The helo");
    assert_eq!(squiggles(&mut s, 0), ["helo"]);
    assert_eq!(reported(&mut s), ["helo"]);
}

#[test]
fn apostrophes_and_hyphens_stay_part_of_the_word_being_typed() {
    // Smart quotes turn the apostrophe into ’, which the spelling check keeps in the word.
    let mut s = typed("It doesn'");
    assert_eq!(s.doc.plain_text(StoryRef::Body), "It doesn’");
    assert_eq!(reported(&mut s), ["doesn"], "the stem alone is a misspelling");
    assert_eq!(squiggles(&mut s, 0), Vec::<String>::new());
    let mut s = typed("It doesn't ");
    assert_eq!(squiggles(&mut s, 0), Vec::<String>::new());
    // A hyphen joins the parts of a compound being typed.
    let mut s = typed("The nonn-");
    assert_eq!(reported(&mut s), ["nonn"]);
    assert_eq!(squiggles(&mut s, 0), Vec::<String>::new());
    typ(&mut s, "exist");
    assert_eq!(squiggles(&mut s, 0), Vec::<String>::new());
    typ(&mut s, " ");
    assert!(!squiggles(&mut s, 0).is_empty());
    assert_eq!(squiggles(&mut s, 0), reported(&mut s));
    // A hyphen after a space starts no word.
    let mut s = typed("The helo -");
    assert_eq!(squiggles(&mut s, 0), ["helo"]);
}

#[test]
fn text_commands_that_edit_nothing_leave_an_existing_word_marked() {
    let mut s = typed("Fine\nThe helo");
    s.sel = crate::Selection::caret(Pos::body(0, 2));
    assert_eq!(squiggles(&mut s, 1), ["helo"]);
    s.sel = crate::Selection::caret(Pos::body(1, 8));
    // Insert switches overtype; Delete at the end of the document and Shift+Tab change no text.
    run(&mut s, "text.overtype", json!({}));
    assert_eq!(squiggles(&mut s, 1), ["helo"], "Insert");
    run(&mut s, "text.overtype", json!({}));
    let _ = s.run("text.delete", &json!({}));
    assert_eq!(squiggles(&mut s, 1), ["helo"], "Delete with nothing after the caret");
    let _ = s.run("text.backTab", &json!({}));
    assert_eq!(squiggles(&mut s, 1), ["helo"], "Shift+Tab");
    assert_eq!(s.doc.plain_text(StoryRef::Body), "Fine\nThe helo");
}

#[test]
fn typing_over_backward_selection_keeps_its_font() {
    // As in Word, text typed over a selection takes the first selected character's formatting,
    // whichever way the selection was made (#2879).
    for (anchor, focus) in [(6, 11), (11, 6)] {
        let mut s = s();
        typ(&mut s, "Hello world");
        run(&mut s, "select.range", json!({"anchor": {"block": 0, "off": 6}, "focus": {"block": 0, "off": 11}}));
        run(&mut s, "format.font", json!({"name": "Courier New"}));
        run(&mut s, "select.range", json!({"anchor": {"block": 0, "off": anchor}, "focus": {"block": 0, "off": focus}}));
        typ(&mut s, "X");
        let p = s.doc.body.first().and_then(|b| b.as_para()).unwrap();
        assert_eq!(p.text, "Hello X", "anchor={anchor} focus={focus}");
        assert_eq!(p.props_of_char(6).font.as_deref(), Some("Courier New"), "anchor={anchor} focus={focus}");
        assert_eq!(p.props_of_char(0).font, None, "anchor={anchor} focus={focus}");
    }
}

#[test]
fn right_arrow_moves_through_a_ligature_one_letter_at_a_time() {
    // A font that draws "ffi" as one glyph (bundled, so the test doesn't depend on the installed
    // fonts); each Right press moves the drawn caret (#1974).
    let mut s = s();
    run(&mut s, "format.font", json!({"name": "Source Serif 4"}));
    typ(&mut s, "ffi");
    let drawn_as_one_glyph = s
        .layout()
        .pages
        .iter()
        .flat_map(|p| p.items.iter())
        .any(|it| matches!(it, wordcraft_layout::Placed::Lines { para, .. } if para.glyphs.len() < para.clusters.len()));
    assert!(drawn_as_one_glyph, "Source Serif 4 should draw ffi as a ligature");
    run(&mut s, "caret.home", json!({}));
    let mut xs = Vec::new();
    for _ in 0..4 {
        let l = s.layout();
        xs.push((s.sel.focus.off, l.caret(&s.sel.focus).unwrap().x));
        run(&mut s, "caret.right", json!({}));
    }
    let offs: Vec<usize> = xs.iter().map(|p| p.0).collect();
    assert_eq!(offs, [0, 1, 2, 3]);
    assert!(xs.windows(2).all(|w| w[0].1 < w[1].1), "caret x after each Right press: {xs:?}");
}

/// Two original paragraphs whose list comes from a style, optionally through a basedOn chain.
fn style_inherited_list(kind: wordcraft_doc::numbering::ListKind, chain: bool) -> Session {
    use wordcraft_doc::props::NumRef;
    use wordcraft_doc::styles::Style;
    let mut doc = wordcraft_doc::Document::from_text("Alpha\nBeta");
    let num = doc.numbering.try_add_list(kind).unwrap();
    doc.styles.styles.push(Style {
        id: "ListBase".into(),
        name: "Original List".into(),
        para: wordcraft_doc::ParaProps { numbering: Some(NumRef { num, level: 0 }), ..Default::default() },
        ..Default::default()
    });
    if chain {
        doc.styles.styles.push(Style {
            id: "ListChild".into(),
            name: "Original Derived".into(),
            based_on: Some("ListBase".into()),
            ..Default::default()
        });
    }
    for i in 0..2 {
        doc.para_mut(StoryRef::Body, &wordcraft_doc::Path::top(i)).unwrap().props.style = Some(if chain { "ListChild" } else { "ListBase" }.into());
    }
    Session::new(doc)
}

#[test]
fn toggle_style_inherited_list_off_preserves_other_paragraphs_and_round_trips() {
    use wordcraft_doc::numbering::ListKind;
    for (kind, command) in [(ListKind::Bullet, "para.bullets"), (ListKind::Numbered, "para.numbering")] {
        for chain in [false, true] {
            let mut s = style_inherited_list(kind, chain);
            let before = s.doc.clone();
            s.sel = crate::Selection::caret(Pos::body(0, 2));
            run(&mut s, command, json!({}));
            let p = s.doc.para_at(&Pos::body(0, 0)).unwrap();
            assert_eq!(p.props.numbering.unwrap().num, 0, "{command}, chain={chain}");
            assert_eq!(p.props.style, before.para_at(&Pos::body(0, 0)).unwrap().props.style);
            assert_eq!(s.doc.para_at(&Pos::body(1, 0)), before.para_at(&Pos::body(1, 0)));
            assert_eq!(s.doc.styles, before.styles);
            assert_eq!(s.doc.numbering, before.numbering);
            let after = s.doc.clone();
            run(&mut s, "edit.undo", json!({}));
            assert_eq!(s.doc, before);
            run(&mut s, "edit.redo", json!({}));
            assert_eq!(s.doc, after);
            let bytes = crate::io::save_bytes("list.docx", &s.doc).unwrap();
            let saved = crate::io::open_bytes("list.docx", &bytes).unwrap();
            let a = saved.para_at(&Pos::body(0, 0)).unwrap();
            let b = saved.para_at(&Pos::body(1, 0)).unwrap();
            assert_eq!(a.props.numbering.unwrap().num, 0);
            assert!(saved.styles.resolve_para(&a.props).numbering.is_none());
            assert!(saved.styles.resolve_para(&b.props).numbering.is_some_and(|n| n.num != 0));
            assert_eq!(a.text, "Alpha");
            assert_eq!(b.text, "Beta");
        }
    }
}

#[test]
fn explicit_list_off_override_is_not_toggled_off_again() {
    use wordcraft_doc::numbering::ListKind;
    use wordcraft_doc::props::NumRef;
    for (kind, command) in [(ListKind::Bullet, "para.bullets"), (ListKind::Numbered, "para.numbering")] {
        let mut s = style_inherited_list(kind, true);
        s.doc.para_mut(StoryRef::Body, &wordcraft_doc::Path::top(0)).unwrap().props.numbering = Some(NumRef { num: 0, level: 0 });
        s.sel = crate::Selection::caret(Pos::body(0, 2));
        run(&mut s, command, json!({}));
        assert!(s.doc.para_at(&Pos::body(0, 0)).unwrap().props.numbering.is_some_and(|n| n.num != 0));
        // A mixed selection must turn the list on, rather than remove the inherited item.
        s.doc.para_mut(StoryRef::Body, &wordcraft_doc::Path::top(0)).unwrap().props.numbering = Some(NumRef { num: 0, level: 0 });
        s.sel = crate::Selection { anchor: Pos::body(0, 0), focus: Pos::body(1, 4) };
        run(&mut s, command, json!({}));
        for i in 0..2 {
            let p = s.doc.para_at(&Pos::body(i, 0)).unwrap();
            assert!(s.doc.styles.resolve_para(&p.props).numbering.is_some_and(|n| n.num != 0));
        }
    }
}

#[test]
fn list_toggle_uses_effective_level_and_override() {
    use wordcraft_doc::numbering::{ListKind, levels_for};
    use wordcraft_doc::props::NumRef;
    for (kind, command, wrong) in [(ListKind::Bullet, "para.bullets", "para.numbering"), (ListKind::Numbered, "para.numbering", "para.bullets")] {
        for override_level in [false, true] {
            for explicit_kind in [false, true] {
                let mut s = style_inherited_list(kind, true);
                let opposite = if kind == ListKind::Bullet { ListKind::Numbered } else { ListKind::Bullet };
                let num = s.doc.styles.styles.iter_mut().find(|x| x.id == "ListBase").unwrap().para.numbering.as_mut().unwrap();
                num.level = 1;
                let num_id = num.num;
                s.doc.numbering.abstracts[0].levels[0] = levels_for(opposite)[0].clone();
                if override_level {
                    s.doc.numbering.abstracts[0].levels[1] = levels_for(opposite)[1].clone();
                    s.doc.numbering.nums[0].level_overrides.push((1, levels_for(kind)[1].clone()));
                }
                let before = s.doc.clone();
                let params = if explicit_kind { json!({"kind": if kind == ListKind::Bullet { "bullet" } else { "numbered" }}) } else { json!({}) };
                run(&mut s, command, params);
                assert_eq!(
                    s.doc.para_at(&Pos::body(0, 0)).unwrap().props.numbering,
                    Some(NumRef { num: 0, level: 0 }),
                    "{command}, override={override_level}, explicit={explicit_kind}"
                );
                assert_eq!(s.doc.styles, before.styles);
                assert_eq!(s.doc.numbering, before.numbering);
                assert_eq!(s.doc.para_at(&Pos::body(1, 0)), before.para_at(&Pos::body(1, 0)));
                let mut other = Session::new(before);
                run(&mut other, wrong, json!({}));
                assert!(other.doc.para_at(&Pos::body(0, 0)).unwrap().props.numbering.is_some_and(|n| n.num != 0));
                assert!(other.doc.numbering.level(num_id, 1).is_some());
            }
        }
    }
}

#[test]
fn list_paragraph_style_survives_inherited_toggle_off() {
    use wordcraft_doc::numbering::ListKind;
    use wordcraft_doc::props::{Align, ParaProps};
    use wordcraft_doc::styles::Style;
    for kind in [ListKind::Bullet, ListKind::Numbered] {
        let mut s = style_inherited_list(kind, false);
        *s.doc.styles.styles.iter_mut().find(|x| x.id == "ListParagraph").unwrap() = Style {
            id: "ListParagraph".into(),
            name: "Original formatted list".into(),
            based_on: Some("ListBase".into()),
            para: ParaProps { align: Some(Align::Right), space_after: Some(17.0), ..Default::default() },
            ..Default::default()
        };
        s.doc.para_mut(StoryRef::Body, &wordcraft_doc::Path::top(0)).unwrap().props.style = Some("ListParagraph".into());
        let before = s.doc.clone();
        run(&mut s, if kind == ListKind::Bullet { "para.bullets" } else { "para.numbering" }, json!({}));
        let p = s.doc.para_at(&Pos::body(0, 0)).unwrap();
        assert_eq!(p.props.style.as_deref(), Some("ListParagraph"));
        let effective = s.doc.styles.resolve_para(&p.props);
        assert!(effective.numbering.is_none());
        assert_eq!(effective.align, Align::Right);
        assert_eq!(effective.space_after, 17.0);
        assert_eq!(s.doc.styles, before.styles);
        assert_eq!(s.doc.numbering, before.numbering);
        assert_eq!(s.doc.para_at(&Pos::body(1, 0)), before.para_at(&Pos::body(1, 0)));
        let after = s.doc.clone();
        run(&mut s, "edit.undo", json!({}));
        assert_eq!(s.doc, before);
        run(&mut s, "edit.redo", json!({}));
        assert_eq!(s.doc, after);
        let bytes = crate::io::save_bytes("list.docx", &s.doc).unwrap();
        let saved = crate::io::open_bytes("list.docx", &bytes).unwrap();
        let p = saved.para_at(&Pos::body(0, 0)).unwrap();
        assert_eq!(p.props.style.as_deref(), Some("ListParagraph"));
        let effective = saved.styles.resolve_para(&p.props);
        assert!(effective.numbering.is_none());
        assert_eq!(effective.align, Align::Right);
        assert_eq!(effective.space_after, 17.0);
    }
}

#[test]
fn direct_list_toggle_still_clears_the_automatic_list_paragraph_style() {
    for command in ["para.bullets", "para.numbering"] {
        let mut s = Session::new(wordcraft_doc::Document::from_text("Direct list"));
        run(&mut s, command, json!({}));
        assert_eq!(s.doc.para_at(&Pos::body(0, 0)).unwrap().props.style.as_deref(), Some("ListParagraph"));
        run(&mut s, command, json!({}));
        let p = s.doc.para_at(&Pos::body(0, 0)).unwrap();
        assert!(p.props.style.is_none());
        assert!(s.doc.styles.resolve_para(&p.props).numbering.is_none());
    }
}

#[test]
fn a_new_list_uses_the_requested_kind_after_a_mixed_format_list() {
    use wordcraft_doc::numbering::{ListKind, levels_for};
    use wordcraft_doc::props::{NumRef, ParaProps};
    for (kind, command) in [(ListKind::Bullet, "para.bullets"), (ListKind::Numbered, "para.numbering")] {
        let mut s = style_inherited_list(kind, false);
        let opposite = if kind == ListKind::Bullet { ListKind::Numbered } else { ListKind::Bullet };
        s.doc.numbering.abstracts[0].levels[0] = levels_for(opposite)[0].clone();
        let num = s.doc.numbering.nums[0].id;
        s.doc.para_mut(StoryRef::Body, &wordcraft_doc::Path::top(0)).unwrap().props.numbering = Some(NumRef { num, level: 1 });
        s.doc.para_mut(StoryRef::Body, &wordcraft_doc::Path::top(1)).unwrap().props = ParaProps::default();
        let before = s.doc.para_at(&Pos::body(0, 0)).unwrap().clone();
        s.sel = crate::Selection::caret(Pos::body(1, 0));
        run(&mut s, command, json!({}));
        let n = s.doc.para_at(&Pos::body(1, 0)).unwrap().props.numbering.unwrap();
        assert_eq!(n.level, 0);
        let level = s.doc.numbering.level(n.num, n.level).unwrap();
        assert_eq!(level.format == wordcraft_doc::section::NumFormat::Bullet, kind == ListKind::Bullet);
        assert_eq!(s.doc.para_at(&Pos::body(0, 0)).unwrap(), &before);
    }
}

/// #4085: the registered shortcut toggles the last selected highlight, while explicit
/// colour commands remain idempotent setters for the palette, CLI and automation.
#[test]
#[cfg(not(target_os = "macos"))]
fn highlight_shortcut_toggles_the_last_colour() {
    let mut s = typed("Alpha Beta");
    run(&mut s, "select.all", json!({}));
    let colour = |s: &mut Session| s.run("format.state", &json!({})).unwrap()["highlight"].clone();
    press(&mut s, "Mod+Alt+H").unwrap();
    assert_eq!(colour(&mut s), json!("FFFF00"));
    press(&mut s, "Mod+Alt+H").unwrap();
    assert_eq!(colour(&mut s), serde_json::Value::Null);
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(colour(&mut s), json!("FFFF00"));
    run(&mut s, "edit.redo", json!({}));
    assert_eq!(colour(&mut s), serde_json::Value::Null);
    run(&mut s, "format.highlight", json!({"color":"blue"}));
    run(&mut s, "format.highlight", json!({"color":"blue"}));
    assert_eq!(colour(&mut s), json!("0000FF"));
    press(&mut s, "Mod+Alt+H").unwrap();
    assert_eq!(colour(&mut s), serde_json::Value::Null);
    press(&mut s, "Mod+Alt+H").unwrap();
    assert_eq!(colour(&mut s), json!("0000FF"));
    run(&mut s, "format.highlight", json!({"color":"none"}));
    press(&mut s, "Mod+Alt+H").unwrap();
    assert_eq!(colour(&mut s), serde_json::Value::Null);
    assert_eq!(s.doc.body[0].as_para().unwrap().text, "Alpha Beta");
}

/// #4085: mixed selections take one colour; removing an inherited colour writes an
/// explicit No Color, and a collapsed caret toggles pending formatting for future typing.
#[test]
fn highlight_toggle_handles_mixed_inherited_and_pending_formatting() {
    let mut s = typed("Alpha Beta ");
    run(&mut s, "select.text", json!({"text":"Alpha"}));
    run(&mut s, "format.highlight", json!({"color":"blue"}));
    run(&mut s, "select.all", json!({}));
    run(&mut s, "format.highlight", json!({}));
    assert_eq!(s.run("format.state", &json!({})).unwrap()["highlight"], json!("0000FF"));
    run(&mut s, "format.highlight", json!({}));
    assert_eq!(s.run("format.state", &json!({})).unwrap()["highlight"], serde_json::Value::Null);
    s.doc.styles.default_chr.highlight = Some(wordcraft_doc::props::Highlight::Blue);
    run(&mut s, "format.highlight", json!({"color":"blue"}));
    run(&mut s, "format.highlight", json!({}));
    assert!(s.doc.body[0].as_para().unwrap().runs.iter().all(|r| r.props.highlight == Some(wordcraft_doc::props::Highlight::None)));
    run(&mut s, "caret.docEnd", json!({}));
    run(&mut s, "format.highlight", json!({}));
    assert_eq!(s.pending.as_ref().unwrap().highlight, Some(wordcraft_doc::props::Highlight::Blue));
    run(&mut s, "format.highlight", json!({}));
    assert_eq!(s.pending.as_ref().unwrap().highlight, Some(wordcraft_doc::props::Highlight::None));
    run(&mut s, "text.insert", json!({"text":"X"}));
    run(&mut s, "select.text", json!({"text":"X"}));
    assert_eq!(s.run("format.state", &json!({})).unwrap()["highlight"], serde_json::Value::Null);
    let before = serde_json::to_value(&s.doc).unwrap();
    assert!(s.run("format.highlight", &json!({"color":123})).is_err());
    assert_eq!(serde_json::to_value(&s.doc).unwrap(), before);
}

/// A differently coloured final paragraph must remain visible to the shortcut beyond
/// the formatting-state query limit, including a style-inherited tail and Undo/Redo.
#[cfg(not(target_os = "macos"))]
#[test]
fn highlight_toggle_checks_past_the_resolved_run_limit() {
    use wordcraft_doc::para::Run;
    use wordcraft_doc::props::{CharProps, Highlight};
    for count in [10_000, 10_001, 10_002] {
        for inherited in [false, true] {
            let mut doc = wordcraft_doc::Document::from_text(&format!("{}\nZ", "x".repeat(count)));
            doc.para_mut(StoryRef::Body, &Pos::body(0, 0).path).unwrap().runs = (0..count)
                .map(|i| Run { len: 1, props: CharProps { highlight: Some(Highlight::Blue), bold: Some(i % 2 == 0), ..Default::default() } })
                .collect();
            if inherited {
                doc.styles.default_chr.highlight = Some(Highlight::Yellow);
            } else {
                doc.para_mut(StoryRef::Body, &Pos::body(1, 0).path).unwrap().runs[0].props.highlight = Some(Highlight::Yellow);
            }
            doc.para_mut(StoryRef::Body, &Pos::body(0, 0).path).unwrap().mark.highlight = Some(Highlight::Blue);
            let original = doc.clone();
            let mut s = Session::new(doc);
            s.last_highlight = Highlight::Blue;
            run(&mut s, "select.all", json!({}));
            press(&mut s, "Mod+Alt+H").unwrap();
            assert!(
                s.doc.body.iter().filter_map(|b| b.as_para()).flat_map(|p| &p.runs).all(|r| r.props.highlight == Some(Highlight::Blue)),
                "count={count}, inherited={inherited}"
            );
            press(&mut s, "Mod+Alt+H").unwrap();
            assert!(s.doc.body.iter().filter_map(|b| b.as_para()).flat_map(|p| &p.runs).all(|r| r.props.highlight == Some(Highlight::None)));
            run(&mut s, "edit.undo", json!({}));
            assert!(s.doc.body[1].as_para().unwrap().runs.iter().all(|r| r.props.highlight == Some(Highlight::Blue)));
            run(&mut s, "edit.undo", json!({}));
            assert_eq!(serde_json::to_value(&s.doc).unwrap(), serde_json::to_value(&original).unwrap());
            run(&mut s, "edit.redo", json!({}));
            assert!(s.doc.body[1].as_para().unwrap().runs.iter().all(|r| r.props.highlight == Some(Highlight::Blue)));
        }
    }
}

/// A command preview uses the real session's remembered colour without changing its
/// document, colour choice or undo history when the preview toggles highlighting off.
#[test]
fn highlight_preview_preserves_the_session_colour_and_document() {
    use wordcraft_doc::props::Highlight;
    let mut s = typed("Alpha");
    run(&mut s, "select.all", json!({}));
    run(&mut s, "format.highlight", json!({"color":"blue"}));
    let original = serde_json::to_value(&s.doc).unwrap();
    let depth = s.undo_depth();
    let (preview, _, _) = s.preview_layout("format.highlight", &json!({})).unwrap();
    assert!(preview.body[0].as_para().unwrap().runs.iter().all(|r| r.props.highlight == Some(Highlight::None)));
    assert_eq!(serde_json::to_value(&s.doc).unwrap(), original);
    assert_eq!(s.last_highlight, Highlight::Blue);
    assert_eq!(s.undo_depth(), depth);
}

/// Whole-cell column selections include paragraph marks, including empty cells and
/// inherited mark colours; adjacent unselected cells and undo/redo keep their formatting.
#[test]
fn highlight_toggle_checks_whole_cell_marks() {
    use wordcraft_doc::props::Highlight;
    use wordcraft_doc::{CharProps, Paragraph};
    for inherited in [false, true] {
        for empty in [false, true] {
            let mut s = s();
            run(&mut s, "insert.table", json!({"rows":2,"cols":2}));
            let (table, _, _) = s.sel.focus.path.cell().unwrap();
            let pos = |row, col| {
                let mut path = table.clone();
                path.0.extend([row, col, 0]);
                Pos::new(StoryRef::Body, path, 0)
            };
            if inherited {
                s.doc.styles.default_chr.highlight = Some(Highlight::Yellow);
            }
            for row in 0..2 {
                for col in 0..2 {
                    let p = s.doc.para_mut(StoryRef::Body, &pos(row, col).path).unwrap();
                    let colour = if col == 0 { Highlight::Blue } else { Highlight::Red };
                    *p = Paragraph::with_text(
                        if empty && row == 1 && col == 0 { "" } else { "AB" },
                        CharProps { highlight: Some(colour), ..Default::default() },
                    );
                    p.mark.highlight = Some(colour);
                    if row == 1 && col == 0 {
                        p.mark.highlight = if inherited { None } else { Some(Highlight::Yellow) };
                    }
                }
            }
            s.last_highlight = Highlight::Blue;
            run(&mut s, "caret.set", json!({"pos":pos(0,0)}));
            run(&mut s, "table.selectColumn", json!({}));
            assert!(!s.cell_range().unwrap().stream);
            let original = serde_json::to_value(&s.doc).unwrap();
            run(&mut s, "format.highlight", json!({}));
            for row in 0..2 {
                let p = s.doc.para_at(&pos(row, 0)).unwrap();
                assert_eq!(p.mark.highlight, Some(Highlight::Blue), "inherited={inherited}, empty={empty}");
                assert!(p.runs.iter().all(|r| r.props.highlight == Some(Highlight::Blue)));
            }
            run(&mut s, "format.highlight", json!({}));
            for row in 0..2 {
                let p = s.doc.para_at(&pos(row, 0)).unwrap();
                assert_eq!(p.mark.highlight, Some(Highlight::None));
                assert!(p.runs.iter().all(|r| r.props.highlight == Some(Highlight::None)));
                let other = s.doc.para_at(&pos(row, 1)).unwrap();
                assert_eq!(other.mark.highlight, Some(Highlight::Red));
                assert!(other.runs.iter().all(|r| r.props.highlight == Some(Highlight::Red)));
            }
            run(&mut s, "edit.undo", json!({}));
            run(&mut s, "edit.undo", json!({}));
            assert_eq!(serde_json::to_value(&s.doc).unwrap(), original);
            run(&mut s, "edit.redo", json!({}));
            assert_eq!(s.doc.para_at(&pos(1, 0)).unwrap().mark.highlight, Some(Highlight::Blue));
        }
    }
}

/// Fully selected marks participate even when the selection has no text runs or an
/// empty paragraph between blue runs; repeated toggles and undo preserve these targets.
#[test]
fn highlight_toggle_checks_ordinary_paragraph_marks() {
    use wordcraft_doc::props::Highlight;
    for text in ["AB", "AB\n\nAB", "\n"] {
        let mut doc = wordcraft_doc::Document::from_text(text);
        doc.styles.default_chr.highlight = Some(Highlight::Blue);
        let paths = doc.para_paths(StoryRef::Body);
        for path in &paths {
            let p = doc.para_mut(StoryRef::Body, path).unwrap();
            p.mark.highlight = Some(Highlight::Blue);
            for r in &mut p.runs {
                r.props.highlight = Some(Highlight::Blue);
            }
        }
        let mismatch = if text == "AB\n\nAB" { 1 } else { 0 };
        doc.para_mut(StoryRef::Body, &paths[mismatch]).unwrap().mark.highlight = Some(Highlight::Yellow);
        let original = serde_json::to_value(&doc).unwrap();
        let mut s = Session::new(doc);
        s.last_highlight = Highlight::Blue;
        run(&mut s, "select.all", json!({}));
        run(&mut s, "format.highlight", json!({}));
        for path in &paths {
            let p = s.doc.para(StoryRef::Body, path).unwrap();
            assert_eq!(p.mark.highlight, Some(Highlight::Blue), "text={text:?}");
            assert!(p.runs.iter().all(|r| r.props.highlight == Some(Highlight::Blue)));
        }
        run(&mut s, "format.highlight", json!({}));
        assert!(paths.iter().all(|p| s.doc.para(StoryRef::Body, p).unwrap().mark.highlight == Some(Highlight::None)));
        run(&mut s, "edit.undo", json!({}));
        run(&mut s, "edit.undo", json!({}));
        assert_eq!(serde_json::to_value(&s.doc).unwrap(), original);
    }
}

/// A partial text column excludes paragraph marks and text beyond its selected offsets,
/// so a different mark colour must not prevent clearing the selected blue characters.
#[test]
fn highlight_partial_column_ignores_unselected_marks() {
    use wordcraft_doc::props::Highlight;
    let mut doc = wordcraft_doc::Document::from_text("AB\nAB");
    for path in doc.para_paths(StoryRef::Body) {
        let p = doc.para_mut(StoryRef::Body, &path).unwrap();
        p.mark.highlight = Some(Highlight::Yellow);
        for r in &mut p.runs {
            r.props.highlight = Some(Highlight::Blue);
        }
    }
    let mut s = Session::new(doc);
    s.last_highlight = Highlight::Blue;
    run(&mut s, "select.column", json!({"anchor":Pos::body(0,0),"focus":Pos::body(1,1)}));
    run(&mut s, "format.highlight", json!({}));
    for row in 0..2 {
        let p = s.doc.para_at(&Pos::body(row, 0)).unwrap();
        assert_eq!(p.props_of_char(0).highlight, Some(Highlight::None));
        assert_eq!(p.props_of_char(1).highlight, Some(Highlight::Blue));
        assert_eq!(p.mark.highlight, Some(Highlight::Yellow));
    }
}

#[test]
fn tab_with_a_whole_paragraph_selected_indents_it() {
    let mut s = typed("Hello world");
    run(&mut s, "select.paragraph", json!({}));
    run(&mut s, "text.tab", json!({}));
    assert_eq!(paras(&s), vec![para("Hello world", None)]);
    assert_eq!(indents(&s).0, Some(36.0));
}

#[test]
fn tab_with_a_partial_selection_still_inserts_a_tab() {
    let mut s = typed("Hello world");
    run(&mut s, "select.text", json!({"text": "ello"}));
    run(&mut s, "text.tab", json!({}));
    assert_eq!(paras(&s), vec![para("H\t world", None)]);
}

/// Tab with a whole paragraph selected indents it as one Undo step: the selection stays while
/// the indent is applied, and Undo restores both the indentation and that same selection. A
/// partial selection still types a tab character (the test above), and Tab in a table cell still
/// navigates (the table tests above).
#[test]
fn tab_with_a_whole_paragraph_selected_undoes_indent_and_selection_together() {
    let mut s = typed("Hello world");
    run(&mut s, "select.paragraph", json!({}));
    let selected = s.sel.clone();
    let depth = s.undo_depth();
    run(&mut s, "text.tab", json!({}));
    assert_eq!(s.doc.plain_text(StoryRef::Body), "Hello world", "Tab indents, it types no tab");
    assert_eq!(s.sel, selected, "the whole paragraph stays selected");
    assert_eq!(indents(&s), (Some(36.0), None));
    assert_eq!(s.undo_depth(), depth + 1, "one paragraph, one Undo step");
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(indents(&s), (None, None), "Undo takes the indent back");
    assert_eq!(s.sel, selected, "and restores the selection the indent was applied to");
    assert_eq!(paras(&s), vec![para("Hello world", None)]);
}

/// #2848: "Add a period with double-space" (an AutoFormat As You Type switch, off by default):
/// a second Space after a word, a number or a closing bracket or quote turns the first into a
/// period. Undo puts the two spaces back; other punctuation, a lone space and a run of three
/// spaces are left as typed.
#[test]
fn double_space_adds_a_period_when_asked() {
    let with_option = |keys: &str| {
        let mut s = s();
        run(&mut s, "tools.autocorrect", json!({"doubleSpacePeriod": true}));
        typ(&mut s, keys);
        s
    };
    let text = |s: &Session| s.doc.plain_text(StoryRef::Body);
    // Off unless asked for.
    assert_eq!(text(&typed("Hello  world")), "Hello  world");
    assert!(!s().prefs.autocorrect.double_space_period);
    // On: the second space makes the period, and the next sentence starts with a capital.
    assert_eq!(text(&with_option("Hello  world")), "Hello. world");
    assert_eq!(text(&with_option("Hello  world ")), "Hello. World ");
    assert_eq!(text(&with_option("See page 5  Then")), "See page 5. Then");
    assert_eq!(text(&with_option("(it is done)  Next")), "(it is done). Next");
    assert_eq!(text(&with_option("He said \"stop\"  Next")), "He said “stop”. Next");
    // Undo brings the two spaces back, in one step.
    let mut t = with_option("Hello  ");
    assert_eq!(text(&t), "Hello. ");
    run(&mut t, "edit.undo", json!({}));
    assert_eq!(text(&t), "Hello  ");
    // Left alone: other punctuation before the spaces, a lone space, spaces at the start of a
    // paragraph or after a tab, and a third space.
    for keys in ["Hello.  x", "Hello,  x", "Hello?  x", "Hello;  x", "Hello:  x", "Hello x", "  x", "\n  x", "a\t  x"] {
        let want = typed(keys);
        assert_eq!(text(&with_option(keys)), text(&want), "{keys:?}");
    }
    assert_eq!(text(&with_option("Hello   x")), "Hello.  x");
    // A selection typed over, and the master switch.
    let mut t = with_option("Hello world");
    run(&mut t, "select.all", json!({}));
    run(&mut t, "text.insert", json!({"text": " "}));
    assert_eq!(text(&t), " ");
    let mut t = s();
    run(&mut t, "tools.autocorrect", json!({"doubleSpacePeriod": true, "enabled": false}));
    typ(&mut t, "Hello  world");
    assert_eq!(text(&t), "Hello  world");
}
