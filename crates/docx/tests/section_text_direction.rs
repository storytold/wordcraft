//! Section text direction (`w:textDirection` in `w:sectPr`, ECMA-376 Part 1 §17.6.20). Fixtures
//! are hand-written from the spec.

use std::io::{Read, Write};

use wordcraft_doc::props::TextDirection;

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
fn section_text_direction_round_trips_in_schema_order() {
    // Three sections: vertical East Asian (tbRl), bottom-to-top (btLr) and a value no one knows.
    let sect = |dir: &str| format!(r#"<w:sectPr><w:titlePg/><w:textDirection w:val="{dir}"/><w:bidi/></w:sectPr>"#);
    let body = format!(
        r#"<w:p><w:pPr>{}</w:pPr><w:r><w:t>縦書き</w:t></w:r></w:p><w:p><w:pPr>{}</w:pPr><w:r><w:t>Up</w:t></w:r></w:p><w:p><w:r><w:t>Odd</w:t></w:r></w:p>{}"#,
        sect("tbRl"),
        sect("btLr"),
        sect("diagonal")
    );
    let d = wordcraft_docx::read(&docx(&body)).unwrap();
    let dirs: Vec<TextDirection> = d.sections().iter().map(|(_, s)| s.text_direction).collect();
    assert_eq!(dirs, vec![TextDirection::Down, TextDirection::Up, TextDirection::Horizontal]);
    let out = wordcraft_docx::write(&d).unwrap();
    let xml = document_xml(&out);
    // After w:titlePg and before w:bidi (CT_SectPr's sequence); a horizontal section writes none.
    assert!(xml.contains(r#"<w:titlePg/><w:textDirection w:val="tbRl"/><w:bidi/>"#), "{xml}");
    assert!(xml.contains(r#"<w:textDirection w:val="btLr"/>"#));
    assert_eq!(xml.matches("w:textDirection").count(), 2);
    let back = wordcraft_docx::read(&out).unwrap();
    let again: Vec<TextDirection> = back.sections().iter().map(|(_, s)| s.text_direction).collect();
    assert_eq!(again, dirs);
}
