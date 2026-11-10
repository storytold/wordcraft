//! Section page-number settings (`w:pgNumType` in `w:sectPr`, ECMA-376 Part 1 §17.6.12).
//! Fixtures are hand-written from the spec.

use std::io::{Read, Write};

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

fn document_xml(bytes: &[u8]) -> String {
    let mut z = zip::ZipArchive::new(std::io::Cursor::new(bytes)).unwrap();
    let mut s = String::new();
    z.by_name("word/document.xml").unwrap().read_to_string(&mut s).unwrap();
    s
}

#[test]
fn page_number_type_is_kept_even_when_it_only_repeats_the_defaults() {
    // A decimal format with no start changes nothing, but the file said it, so saving says it too.
    let body = r#"<w:p><w:r><w:t>One</w:t></w:r></w:p><w:sectPr><w:pgNumType w:fmt="decimal"/></w:sectPr>"#;
    let d = wordcraft_docx::read(&docx(body)).unwrap();
    let xml = document_xml(&wordcraft_docx::write(&d).unwrap());
    assert!(xml.contains(r#"<w:pgNumType w:fmt="decimal"/>"#), "{xml}");
}

#[test]
fn page_number_type_with_a_start_and_format_still_round_trips() {
    let body = r#"<w:p><w:r><w:t>One</w:t></w:r></w:p><w:sectPr><w:pgNumType w:fmt="upperRoman" w:start="3"/></w:sectPr>"#;
    let d = wordcraft_docx::read(&docx(body)).unwrap();
    let xml = document_xml(&wordcraft_docx::write(&d).unwrap());
    assert!(xml.contains(r#"w:fmt="upperRoman""#) && xml.contains(r#"w:start="3""#), "{xml}");
}

#[test]
fn a_section_without_page_number_type_writes_none() {
    let body = r#"<w:p><w:r><w:t>One</w:t></w:r></w:p><w:sectPr/>"#;
    let d = wordcraft_docx::read(&docx(body)).unwrap();
    assert!(!document_xml(&wordcraft_docx::write(&d).unwrap()).contains("pgNumType"));
}
