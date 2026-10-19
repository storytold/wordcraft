//! Page borders (`w:pgBorders`, ECMA-376 §17.6.10) keep `w:offsetFrom`, `w:display` and
//! `w:zOrder` through a round trip.

use std::io::{Read, Write};

use wordcraft_doc::props::{Border, Borders};

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

fn side(n: &str) -> String {
    format!(r#"<w:{n} w:val="single" w:sz="8" w:space="24" w:color="000000"/>"#)
}

#[test]
fn page_border_attributes_round_trip() {
    let sides: String = ["top", "left", "bottom", "right"].iter().map(|n| side(n)).collect();
    let body = format!(r#"<w:p/><w:sectPr><w:pgBorders w:display="firstPage" w:offsetFrom="page" w:zOrder="back">{sides}</w:pgBorders></w:sectPr>"#);
    let d = wordcraft_docx::read(&docx(&body)).unwrap();
    let xml = document_xml(&wordcraft_docx::write(&d).unwrap());
    assert!(xml.contains(r#"w:display="firstPage""#), "{xml}");
    assert!(xml.contains(r#"w:offsetFrom="page""#), "{xml}");
    assert!(xml.contains(r#"w:zOrder="back""#), "{xml}");
}

#[test]
fn text_offset_is_kept() {
    let body = format!(r#"<w:p/><w:sectPr><w:pgBorders w:offsetFrom="text">{}</w:pgBorders></w:sectPr>"#, side("top"));
    let d = wordcraft_docx::read(&docx(&body)).unwrap();
    let xml = document_xml(&wordcraft_docx::write(&d).unwrap());
    assert!(xml.contains(r#"w:offsetFrom="text""#), "{xml}");
    assert!(!xml.contains("w:display") && !xml.contains("w:zOrder"), "{xml}");
}

#[test]
fn new_page_border_is_measured_from_the_page_edge() {
    let mut d = wordcraft_docx::read(&docx("<w:p/>")).unwrap();
    d.last_section.page_borders = Some(Borders::box_(Border::single(1.0)));
    let xml = document_xml(&wordcraft_docx::write(&d).unwrap());
    assert!(xml.contains(r#"<w:pgBorders w:offsetFrom="page">"#), "{xml}");
}

#[test]
fn missing_offset_from_is_not_written_as_page() {
    let body = format!(r#"<w:p/><w:sectPr><w:pgBorders>{}</w:pgBorders></w:sectPr>"#, side("top"));
    let d = wordcraft_docx::read(&docx(&body)).unwrap();
    let xml = document_xml(&wordcraft_docx::write(&d).unwrap());
    assert!(!xml.contains(r#"w:offsetFrom="page""#), "{xml}");
}
