//! Indents in character units (#534), common in Chinese documents: ECMA-376 Part 1 §17.3.1.12
//! `w:ind` `w:firstLineChars`, `w:hangingChars`, `w:leftChars` and `w:rightChars`, in hundredths
//! of a character. Fixtures are hand-written.

use std::io::{Read, Write};

use wordcraft_doc::Document;
use wordcraft_doc::props::ParaProps;

fn zip(entries: &[(&str, &[u8])]) -> Vec<u8> {
    let mut zw = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    for (name, bytes) in entries {
        zw.start_file(*name, zip::write::SimpleFileOptions::default()).unwrap();
        zw.write_all(bytes).unwrap();
    }
    zw.finish().unwrap().into_inner()
}

/// A package whose body is `body`.
fn docx(body: &str) -> Vec<u8> {
    let root = r#"<?xml version="1.0" encoding="UTF-8"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="word/document.xml"/></Relationships>"#;
    let doc = format!(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:body>{body}</w:body></w:document>"#
    );
    zip(&[("_rels/.rels", root.as_bytes()), ("word/document.xml", doc.as_bytes())])
}

fn props(d: &Document) -> Vec<ParaProps> {
    d.body.iter().filter_map(|b| b.as_para()).map(|p| p.props.clone()).collect()
}

#[test]
fn character_indents_read_and_round_trip() {
    let body = r#"<w:p><w:pPr><w:ind w:firstLineChars="200" w:firstLine="420"/></w:pPr><w:r><w:t>一</w:t></w:r></w:p>
        <w:p><w:pPr><w:ind w:leftChars="150" w:left="315" w:rightChars="50" w:right="105" w:hangingChars="100" w:hanging="210"/></w:pPr><w:r><w:t>二</w:t></w:r></w:p>
        <w:p><w:pPr><w:ind w:firstLineChars="0" w:firstLine="240"/></w:pPr><w:r><w:t>三</w:t></w:r></w:p>
        <w:p><w:pPr><w:ind w:firstLineChars="99999999" w:leftChars="junk"/></w:pPr><w:r><w:t>四</w:t></w:r></w:p>"#;
    let d = wordcraft_docx::read(&docx(body)).unwrap();
    let p = props(&d);
    assert_eq!((p[0].indent_first_chars, p[0].indent_first), (Some(2.0), Some(21.0)));
    assert_eq!((p[1].indent_left_chars, p[1].indent_right_chars, p[1].indent_first_chars), (Some(1.5), Some(0.5), Some(-1.0)));
    // Zero cancels an inherited value; the points apply.
    assert_eq!((p[2].indent_first_chars, p[2].indent_first), (Some(0.0), Some(12.0)));
    // Hostile values are clamped or dropped.
    assert_eq!((p[3].indent_first_chars, p[3].indent_left_chars), (Some(1584.0), None));

    let mut z = zip::ZipArchive::new(std::io::Cursor::new(wordcraft_docx::write(&d).unwrap())).unwrap();
    let mut xml = String::new();
    z.by_name("word/document.xml").unwrap().read_to_string(&mut xml).unwrap();
    for attr in [r#"w:firstLineChars="200""#, r#"w:leftChars="150""#, r#"w:rightChars="50""#, r#"w:hangingChars="100""#, r#"w:firstLineChars="0""#] {
        assert!(xml.contains(attr), "{attr} missing: {xml}");
    }
    let again = props(&wordcraft_docx::read(&docx(&xml[xml.find("<w:body>").unwrap() + 8..xml.find("</w:body>").unwrap()])).unwrap());
    assert_eq!(again[..3], p[..3]);
}

/// HTML-style automatic paragraph spacing (`w:beforeAutospacing`, `w:afterAutospacing`, ECMA-376
/// §17.3.1.33), common in text pasted from web pages: read, "0" switching an inherited one off,
/// and written back.
#[test]
fn automatic_spacing_reads_and_round_trips() {
    let body = r#"<w:p><w:pPr><w:spacing w:before="100" w:beforeAutospacing="1" w:after="100" w:afterAutospacing="1"/></w:pPr><w:r><w:t>a</w:t></w:r></w:p>
        <w:p><w:pPr><w:spacing w:before="0" w:beforeAutospacing="0" w:after="100" w:afterAutospacing="true"/></w:pPr><w:r><w:t>b</w:t></w:r></w:p>
        <w:p><w:pPr><w:spacing w:after="100"/></w:pPr><w:r><w:t>c</w:t></w:r></w:p>"#;
    let d = wordcraft_docx::read(&docx(body)).unwrap();
    let p = props(&d);
    assert_eq!((p[0].space_before_auto, p[0].space_after_auto, p[0].space_before), (Some(true), Some(true), Some(5.0)));
    assert_eq!((p[1].space_before_auto, p[1].space_after_auto), (Some(false), Some(true)));
    assert_eq!((p[2].space_before_auto, p[2].space_after_auto), (None, None));
    let rp = d.styles.resolve_para(&p[0]);
    assert_eq!((rp.space_before, rp.space_after, rp.before_auto), (14.0, 14.0, true), "automatic spacing is 14 pt");
    assert_eq!(d.styles.resolve_para(&p[2]).space_after, 5.0);

    let mut z = zip::ZipArchive::new(std::io::Cursor::new(wordcraft_docx::write(&d).unwrap())).unwrap();
    let mut xml = String::new();
    z.by_name("word/document.xml").unwrap().read_to_string(&mut xml).unwrap();
    for attr in [r#"w:beforeAutospacing="1""#, r#"w:afterAutospacing="1""#, r#"w:beforeAutospacing="0""#] {
        assert!(xml.contains(attr), "{attr} missing");
    }
    let again = wordcraft_docx::read(&wordcraft_docx::write(&d).unwrap()).unwrap();
    assert_eq!(props(&again)[..3], p[..3]);
}
