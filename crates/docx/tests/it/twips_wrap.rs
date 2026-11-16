//! Paragraph indents and spacing written as plain twips beyond 16 bits: Word keeps only the low
//! 16 bits (ECMA-376 Part 1 §17.3.1.12 `w:ind`, §17.3.1.33 `w:spacing`). Measured in Word: a
//! `w:left` of 66256 starts the text 36 pt in, a `w:right` of 66976 leaves a 72 pt right indent.
//! Fixtures are hand-written.

use wordcraft_doc::Document;
use wordcraft_doc::props::ParaProps;

use crate::fixtures::zip;

/// A package whose body is `body`.
fn docx(body: &str) -> Vec<u8> {
    let root = r#"<?xml version="1.0" encoding="UTF-8"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="word/document.xml"/></Relationships>"#;
    let doc = format!(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:body>{body}</w:body></w:document>"#
    );
    zip(&[("_rels/.rels", root.as_bytes()), ("word/document.xml", doc.as_bytes())])
}

fn props(body: &str) -> Vec<ParaProps> {
    let d: Document = wordcraft_docx::read(&docx(body)).unwrap();
    d.body.iter().filter_map(|b| b.as_para()).map(|p| p.props.clone()).collect()
}

#[test]
fn indents_beyond_16_bits_wrap() {
    let p = props(
        r#"<w:p><w:pPr><w:ind w:left="66256"/></w:pPr><w:r><w:t>a</w:t></w:r></w:p>
        <w:p><w:pPr><w:ind w:right="66976"/></w:pPr><w:r><w:t>b</w:t></w:r></w:p>
        <w:p><w:pPr><w:ind w:start="65556" w:end="-65516"/></w:pPr><w:r><w:t>c</w:t></w:r></w:p>
        <w:p><w:pPr><w:ind w:left="98304" w:hanging="65896"/></w:pPr><w:r><w:t>d</w:t></w:r></w:p>"#,
    );
    assert_eq!(p[0].indent_left, Some(36.0));
    assert_eq!(p[1].indent_right, Some(72.0));
    // 65556 -> 20 twips; -65516 -> 20 twips.
    assert_eq!((p[2].indent_left, p[2].indent_right), (Some(1.0), Some(1.0)));
    // 98304 = 0x18000 -> 0x8000 = -32768 as a signed 16-bit value; hanging 65896 -> 360.
    assert_eq!((p[3].indent_left, p[3].indent_first), (Some(-1638.4), Some(-18.0)));
}

#[test]
fn spacing_beyond_16_bits_wraps() {
    let p = props(r#"<w:p><w:pPr><w:spacing w:before="65537" w:after="65776"/></w:pPr><w:r><w:t>a</w:t></w:r></w:p>"#);
    assert_eq!((p[0].space_before, p[0].space_after), (Some(0.05), Some(12.0)));
}

#[test]
fn in_range_and_unit_values_are_unchanged() {
    let p = props(
        r#"<w:p><w:pPr><w:ind w:left="32767" w:right="-32768"/><w:spacing w:before="65535"/></w:pPr><w:r><w:t>a</w:t></w:r></w:p>
        <w:p><w:pPr><w:ind w:left="1in" w:right="720"/><w:spacing w:before="12pt"/></w:pPr><w:r><w:t>b</w:t></w:r></w:p>"#,
    );
    assert_eq!((p[0].indent_left, p[0].indent_right, p[0].space_before), (Some(1638.35), Some(-1638.4), Some(3276.75)));
    assert_eq!((p[1].indent_left, p[1].indent_right, p[1].space_before), (Some(72.0), Some(36.0), Some(12.0)));
}
