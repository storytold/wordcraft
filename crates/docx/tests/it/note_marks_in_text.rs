//! A footnote's own mark (`w:footnoteRef`) outside any footnote: Word shows the number 1 there,
//! in the run's own formatting. Measured in Word for Windows 16.0.20625 on synthetic documents
//! (body text and a table cell, with no footnotes and with footnotes before and after the mark,
//! and with lower-case Roman or upper-case letter numbers set in `w:settings`, which Word does
//! not apply to it). All markup here is synthetic, written from ECMA-376 §17.11.

use wordcraft_doc::props::VertAlign;
use wordcraft_doc::{Block, Document, InlineObject};

use crate::fixtures::zip;

const W_NS: &str = r#"xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships""#;
const ROOT_RELS: &str = r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="word/document.xml"/></Relationships>"#;
const DOC_RELS: &str = r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/footnotes" Target="footnotes.xml"/><Relationship Id="rId2" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/settings" Target="settings.xml"/></Relationships>"#;

/// A document whose body is `body`, with one footnote and `settings` inside `w:settings`.
fn read(body: &str, settings: &str) -> Document {
    let doc = format!(r#"<w:document {W_NS}><w:body>{body}</w:body></w:document>"#);
    let footnotes = format!(
        r#"<w:footnotes {W_NS}><w:footnote w:id="1"><w:p><w:r><w:footnoteRef/></w:r><w:r><w:t xml:space="preserve"> Foot.</w:t></w:r></w:p></w:footnote></w:footnotes>"#
    );
    let settings = format!(r#"<w:settings {W_NS}>{settings}</w:settings>"#);
    let bytes = zip(&[
        ("_rels/.rels", ROOT_RELS.as_bytes()),
        ("word/document.xml", doc.as_bytes()),
        ("word/_rels/document.xml.rels", DOC_RELS.as_bytes()),
        ("word/footnotes.xml", footnotes.as_bytes()),
        ("word/settings.xml", settings.as_bytes()),
    ]);
    wordcraft_docx::read(&bytes).unwrap()
}

fn body_text(d: &Document, block: usize) -> String {
    match d.body.get(block).map(|b| &**b) {
        Some(Block::Para(p)) => p.text.clone(),
        Some(Block::Table(t)) => t.rows[0].cells[0].blocks[0].as_para().map(|p| p.text.clone()).unwrap_or_default(),
        None => String::new(),
    }
}

const MARKED: &str = r#"<w:p><w:r><w:t xml:space="preserve">Alpha </w:t></w:r><w:r><w:rPr><w:vertAlign w:val="superscript"/></w:rPr><w:footnoteRef/></w:r><w:r><w:t>, omega</w:t></w:r></w:p>"#;

#[test]
fn a_footnote_mark_in_body_text_shows_the_number_one() {
    let cell = format!(r#"<w:tbl><w:tblGrid><w:gridCol w:w="4000"/></w:tblGrid><w:tr><w:tc>{MARKED}</w:tc></w:tr></w:tbl>"#);
    let d = read(&format!("{MARKED}{cell}<w:p/>"), "");
    // In the body and in a table cell: the number 1, as text in the mark's run.
    assert_eq!(body_text(&d, 0), "Alpha 1, omega");
    assert_eq!(body_text(&d, 1), "Alpha 1, omega");
    let Some(Block::Para(p)) = d.body.first().map(|b| &**b) else { panic!("a paragraph") };
    assert!(p.objects.is_empty(), "no note reference without a note");
    let mark = p.props_of_char(p.text.find('1').unwrap());
    assert_eq!(mark.vert_align, Some(VertAlign::Superscript), "the mark keeps its run's formatting");
    // Saved and opened again it is still there.
    let again = wordcraft_docx::read(&wordcraft_docx::write(&d).unwrap()).unwrap();
    assert_eq!(body_text(&again, 0), "Alpha 1, omega");
}

#[test]
fn a_footnote_mark_in_body_text_ignores_the_documents_number_format() {
    // Word shows "1" whatever format and first number `w:settings` gives footnotes.
    for format in ["lowerRoman", "upperLetter"] {
        let d = read(MARKED, &format!(r#"<w:footnotePr><w:numFmt w:val="{format}"/><w:numStart w:val="4"/></w:footnotePr>"#));
        assert_eq!(body_text(&d, 0), "Alpha 1, omega", "{format}");
    }
}

#[test]
fn a_footnotes_own_mark_is_still_its_number() {
    // Inside the footnote the mark is the note's reference to itself, not text.
    let d = read(r#"<w:p><w:r><w:t>Body</w:t></w:r><w:r><w:footnoteReference w:id="1"/></w:r></w:p>"#, "");
    let note = d
        .parts
        .values()
        .find(|p| p.kind == wordcraft_doc::PartKind::Footnote)
        .and_then(|p| p.blocks.first())
        .and_then(|b| b.as_para())
        .expect("the footnote's paragraph");
    assert!(matches!(note.objects.first(), Some(InlineObject::NoteRef { .. })), "the note's own number");
    assert!(!note.text.contains('1'), "not text: {:?}", note.text);
}
