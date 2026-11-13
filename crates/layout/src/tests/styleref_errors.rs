//! STYLEREF error texts, as Word shows them (measured with synthetic documents): no style name,
//! a style the document doesn't have, and a style no text uses.

use crate::*;
use wordcraft_doc::para::InlineObject;
use wordcraft_doc::props::CharProps;
use wordcraft_doc::{PartKind, para_block};

const NO_TEXT: &str = "Error! No text of specified style in document.";
const NO_NAME: &str = "Error! No style name given.";

fn not_defined(name: &str) -> String {
    format!("Error! Use the Home tab to apply {name} to the text that you want to appear here.")
}

fn field(instr: &str) -> InlineObject {
    InlineObject::Field { instr: instr.into(), result: "stale".into(), locked: false, dirty: false, code: false, form: String::new() }
}

/// A body of one `Heading1` paragraph and one `Normal` paragraph, and a header with one
/// STYLEREF per code; returns what each header field shows on page 1.
fn header_results(codes: &[&str]) -> Vec<String> {
    crate::test_fonts();
    let mut d = Document::from_text("Chapter One\nBody text.");
    d.para_mut(StoryRef::Body, &Path::top(0)).unwrap().props.style = Some("Heading1".to_string());
    d.para_mut(StoryRef::Body, &Path::top(1)).unwrap().props.style = Some("Normal".to_string());
    let blocks = codes
        .iter()
        .map(|c| {
            let mut p = Paragraph::new();
            p.insert_object(0, field(c), &CharProps::default()).unwrap();
            para_block(p)
        })
        .collect();
    let id = d.add_part(PartKind::Header, blocks);
    d.last_section.headers.default = Some(id);
    let mut c = LayoutCache::new();
    let out = layout(&d, &mut c, &LayoutOptions::default());
    let page = out.pages.first().expect("a page");
    page.header
        .iter()
        .flat_map(|it| if let Placed::Lines { para, .. } = it { para.shown.iter().map(|s| s.1.clone()).collect() } else { vec![] })
        .collect()
}

#[test]
fn a_style_with_text_still_shows_it() {
    assert_eq!(header_results(&[r#"STYLEREF "Heading 1""#, "STYLEREF 1"]), ["Chapter One", "Chapter One"]);
}

#[test]
fn a_style_no_paragraph_uses_says_no_text_of_specified_style() {
    let got = header_results(&[r#"STYLEREF "Heading 2""#, r#"STYLEREF "Heading 2" \n"#, r#"STYLEREF "Heading 2" \l"#]);
    assert_eq!(got, [NO_TEXT, NO_TEXT, NO_TEXT]);
}

#[test]
fn a_heading_level_with_no_heading_says_no_text_of_specified_style() {
    assert_eq!(header_results(&["STYLEREF 2", "STYLEREF 9"]), [NO_TEXT, NO_TEXT]);
}

#[test]
fn a_style_the_document_does_not_have_names_it_as_typed() {
    let got =
        header_results(&[r#"STYLEREF "Nonexistent""#, r#"STYLEREF "nO sUCH Style""#, r#"STYLEREF "Nonexistent" \n"#, r#"STYLEREF "Nonexistent" \p"#]);
    assert_eq!(got, [not_defined("Nonexistent"), not_defined("nO sUCH Style"), not_defined("Nonexistent"), not_defined("Nonexistent")]);
}

#[test]
fn a_number_that_is_not_a_heading_level_is_a_style_name() {
    assert_eq!(header_results(&["STYLEREF 12", "STYLEREF 0"]), [not_defined("12"), not_defined("0")]);
}

#[test]
fn no_style_name_says_so() {
    assert_eq!(header_results(&["STYLEREF", r#"STYLEREF """#, r"STYLEREF \n"]), [NO_NAME, NO_NAME, NO_NAME]);
}

#[test]
fn a_very_long_style_name_is_cut_at_255_characters() {
    let long = "x".repeat(300);
    let code = format!(r#"STYLEREF "{long}""#);
    assert_eq!(header_results(&[code.as_str()]), [not_defined(&"x".repeat(255))]);
}

#[test]
fn the_public_constants_carry_words_texts() {
    assert_eq!(styleref::NO_TEXT, NO_TEXT);
}
