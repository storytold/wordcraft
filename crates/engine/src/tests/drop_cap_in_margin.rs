//! Insert › Drop Cap › In margin with indents (#4173). Observed in Word: a first-line or hanging
//! indent leaves the text paragraph and stays only on the drop cap's own frame paragraph
//! (`w:framePr w:dropCap="margin"`, ECMA-376 Part 1 §17.3.1.11), so every text line starts at the
//! same position; a left indent stays on the text and is copied to the cap's paragraph; the
//! alignment is copied to the cap's paragraph and the text keeps it. Dropped is unchanged.

use std::io::Read;

use serde_json::{Value, json};

use crate::Session;

fn run(s: &mut Session, cmd: &str, v: Value) -> Value {
    s.run(cmd, &v).unwrap_or_else(|e| panic!("{cmd} {v}: {e:?}"))
}

fn session(indents: Value) -> Session {
    let mut s = Session::new(wordcraft_doc::Document::new());
    run(&mut s, "text.insert", json!({"text": "Once upon a time there was a letter"}));
    run(&mut s, "para.indents", indents);
    s
}

/// (left, firstLine) of the body's first paragraph, points (unset = 0).
fn text_indents(s: &Session) -> (f32, f32) {
    let p = s.doc.body.first().and_then(|b| b.as_para()).map(|p| p.props.clone()).unwrap_or_default();
    (p.indent_left.unwrap_or(0.0), p.indent_first.unwrap_or(0.0))
}

fn document_xml(bytes: &[u8]) -> String {
    let mut zip = zip::ZipArchive::new(std::io::Cursor::new(bytes)).unwrap();
    let mut xml = String::new();
    zip.by_name("word/document.xml").unwrap().read_to_string(&mut xml).unwrap();
    xml
}

/// Each saved body paragraph's `w:pPr`, as written.
fn ppr_of_paragraphs(xml: &str) -> Vec<String> {
    xml.split("<w:p ")
        .chain(xml.split("<w:p>"))
        .filter(|chunk| chunk.contains("</w:p>"))
        .map(|chunk| chunk.split("</w:p>").next().unwrap_or(""))
        .map(|p| p.split("<w:pPr>").nth(1).and_then(|r| r.split("</w:pPr>").next()).unwrap_or("").to_string())
        .collect()
}

/// (cap paragraph's pPr, text paragraph's pPr) in the saved file.
fn saved(s: &Session) -> (String, String) {
    let xml = document_xml(&wordcraft_docx::write(&s.doc).unwrap());
    let pprs = ppr_of_paragraphs(&xml);
    let cap = pprs.iter().find(|p| p.contains("w:dropCap=\"margin\"")).cloned().unwrap_or_else(|| panic!("no margin cap paragraph in {xml}"));
    let text = pprs.iter().find(|p| !p.contains("w:framePr") && !p.contains("w:sectPr")).cloned().unwrap_or_default();
    (cap, text)
}

/// The `w:ind` element of a pPr (empty when there is none).
fn ind(ppr: &str) -> String {
    ppr.split("<w:ind ").nth(1).and_then(|r| r.split("/>").next()).map(|a| a.trim().to_string()).unwrap_or_default()
}

fn has_attr(ind: &str, attr: &str) -> bool {
    ind.split_whitespace().any(|a| a == attr)
}

#[test]
fn margin_moves_a_hanging_indent_to_the_cap_paragraph() {
    // Left 1.27 cm, hanging 1.27 cm.
    let mut s = session(json!({"left": 36.0, "firstLine": -36.0}));
    run(&mut s, "insert.dropCap", json!({"position": "margin"}));
    assert_eq!(text_indents(&s), (36.0, 0.0), "all text lines start at the left indent");
    let (cap, text) = saved(&s);
    let (cap_ind, text_ind) = (ind(&cap), ind(&text));
    assert!(has_attr(&cap_ind, "w:left=\"720\"") || has_attr(&cap_ind, "w:start=\"720\""), "cap ind: {cap_ind}");
    assert!(has_attr(&cap_ind, "w:hanging=\"720\""), "the hanging indent stays on the cap's paragraph: {cap_ind}");
    assert!(has_attr(&text_ind, "w:left=\"720\"") || has_attr(&text_ind, "w:start=\"720\""), "text ind: {text_ind}");
    assert!(!text_ind.contains("w:hanging=\"7") && !text_ind.contains("w:firstLine=\"7"), "no first-line indent on the text: {text_ind}");
}

#[test]
fn margin_moves_a_first_line_indent_to_the_cap_paragraph() {
    let mut s = session(json!({"firstLine": 36.0}));
    run(&mut s, "insert.dropCap", json!({"position": "margin"}));
    assert_eq!(text_indents(&s), (0.0, 0.0));
    let (cap, text) = saved(&s);
    assert!(has_attr(&ind(&cap), "w:firstLine=\"720\""), "cap ind: {}", ind(&cap));
    assert!(!ind(&text).contains("w:firstLine=\"7"), "text ind: {}", ind(&text));
}

#[test]
fn margin_keeps_a_left_indent_on_both_paragraphs() {
    let mut s = session(json!({"left": 72.0}));
    run(&mut s, "insert.dropCap", json!({"position": "margin"}));
    assert_eq!(text_indents(&s), (72.0, 0.0));
    let (cap, text) = saved(&s);
    for (which, i) in [("cap", ind(&cap)), ("text", ind(&text))] {
        assert!(has_attr(&i, "w:left=\"1440\"") || has_attr(&i, "w:start=\"1440\""), "{which} ind: {i}");
    }
}

#[test]
fn margin_copies_the_alignment_to_the_cap_paragraph() {
    let mut s = session(json!({}));
    run(&mut s, "para.alignCenter", json!({}));
    run(&mut s, "insert.dropCap", json!({"position": "margin"}));
    let (cap, text) = saved(&s);
    assert!(cap.contains("<w:jc w:val=\"center\"/>"), "cap pPr: {cap}");
    assert!(text.contains("<w:jc w:val=\"center\"/>"), "the text keeps its alignment: {text}");
}

#[test]
fn the_cap_paragraphs_indent_survives_save_and_reopen() {
    let mut s = session(json!({"left": 36.0, "firstLine": -36.0}));
    run(&mut s, "insert.dropCap", json!({"position": "margin"}));
    let first = saved(&s);
    let reopened = Session::new(wordcraft_docx::read(&wordcraft_docx::write(&s.doc).unwrap()).unwrap());
    assert_eq!(text_indents(&reopened), (36.0, 0.0), "the text paragraph reopens without the hanging indent");
    assert_eq!(saved(&reopened), first, "saved again, both paragraphs are written the same");
}

/// Guard: Dropped is not part of #4173 and keeps today's behaviour.
#[test]
fn dropped_leaves_the_indents_alone() {
    let mut s = session(json!({"left": 36.0, "firstLine": -36.0}));
    run(&mut s, "insert.dropCap", json!({"position": "dropped"}));
    assert_eq!(text_indents(&s), (36.0, -36.0));
}
