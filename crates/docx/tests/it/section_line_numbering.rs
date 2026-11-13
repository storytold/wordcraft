//! Section line numbering (`w:lnNumType` in `w:sectPr`, ECMA-376 Part 1 §17.6.8).

use std::io::{Read, Write};

use wordcraft_doc::section::{LineNumberRestart, LineNumbering};

/// Builds a minimal in-memory DOCX package archive containing the given XML body.
fn docx(body: &str) -> Vec<u8> {
    let root = r#"<?xml version="1.0" encoding="UTF-8"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="word/document.xml"/></Relationships>"#;
    let doc = format!(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:body>{body}</w:body></w:document>"#
    );
    let mut zw = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    for (name, bytes) in [("_rels/.rels", root.as_bytes()), ("word/document.xml", doc.as_bytes())] {
        zw.start_file(name, zip::write::SimpleFileOptions::default()).unwrap();
        zw.write_all(bytes).unwrap();
    }
    zw.finish().unwrap().into_inner()
}

/// Extracts the `word/document.xml` content string from a serialized DOCX package.
fn document_xml(bytes: &[u8]) -> String {
    let mut z = zip::ZipArchive::new(std::io::Cursor::new(bytes)).unwrap();
    let mut s = String::new();
    z.by_name("word/document.xml").unwrap().read_to_string(&mut s).unwrap();
    s
}

/// #6209: `<w:lnNumType>` without `w:countBy` leaves line numbering off, as Word does.
#[test]
fn section_line_numbering_without_count_by_is_off() {
    let body = r#"<w:p><w:r><w:t>Hello</w:t></w:r></w:p><w:sectPr><w:lnNumType w:distance="0"/></w:sectPr>"#;
    let d = wordcraft_docx::read(&docx(body)).unwrap();
    let sect = &d.last_section;
    assert_eq!(sect.line_numbers, None);

    let out = wordcraft_docx::write(&d).unwrap();
    let xml = document_xml(&out);
    assert!(!xml.contains("w:lnNumType"), "expected no lnNumType when line numbering is off: {xml}");
}

/// #6209: `w:countBy="0"` counts no lines, so line numbering stays off. Word was observed with
/// the attribute absent; an interval of 0 counts nothing either.
#[test]
fn section_line_numbering_with_zero_count_by_is_off() {
    let body = r#"<w:p><w:r><w:t>Hello</w:t></w:r></w:p><w:sectPr><w:lnNumType w:countBy="0" w:distance="0"/></w:sectPr>"#;
    let d = wordcraft_docx::read(&docx(body)).unwrap();
    let sect = &d.last_section;
    assert_eq!(sect.line_numbers, None);

    let out = wordcraft_docx::write(&d).unwrap();
    let xml = document_xml(&out);
    assert!(!xml.contains("w:lnNumType"), "expected no lnNumType when line numbering is off: {xml}");
}

/// #6209: `w:countBy >= 1` enables line numbering.
#[test]
fn section_line_numbering_with_count_by_is_on() {
    let body = r#"<w:p><w:r><w:t>Hello</w:t></w:r></w:p><w:sectPr><w:lnNumType w:countBy="1" w:distance="0" w:start="0" w:restart="newSection"/></w:sectPr>"#;
    let d = wordcraft_docx::read(&docx(body)).unwrap();
    let sect = &d.last_section;
    assert_eq!(sect.line_numbers, Some(LineNumbering { count_by: 1, start: 1, distance: 0.0, restart: LineNumberRestart::Section }));

    let out = wordcraft_docx::write(&d).unwrap();
    let xml = document_xml(&out);
    assert!(xml.contains("w:lnNumType"), "expected lnNumType when line numbering is on: {xml}");
    assert!(xml.contains(r#"w:countBy="1""#), "expected countBy in xml: {xml}");
}
