//! Row-level table property exceptions (`w:tblPrEx` in `w:tr`, ECMA-376 Part 1 §17.4.61).
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

fn row(ex: &str, text: &str) -> String {
    format!(
        r#"<w:tr>{ex}<w:trPr><w:cantSplit/></w:trPr><w:tc><w:tcPr><w:tcW w:w="2000" w:type="dxa"/></w:tcPr><w:p><w:r><w:t>{text}</w:t></w:r></w:p></w:tc></w:tr>"#
    )
}

#[test]
fn row_property_exceptions_round_trip_before_the_row_properties() {
    let ex = r#"<w:tblPrEx><w:tblW w:w="3000" w:type="dxa"/><w:jc w:val="center"/><w:tblInd w:w="240" w:type="dxa"/><w:tblLayout w:type="fixed"/></w:tblPrEx>"#;
    let body = format!(r#"<w:tbl><w:tblPr/><w:tblGrid><w:gridCol w:w="2000"/></w:tblGrid>{}{}</w:tbl><w:p/>"#, row(ex, "A"), row("", "B"));
    let d = wordcraft_docx::read(&docx(&body)).unwrap();
    let out = wordcraft_docx::write(&d).unwrap();
    let xml = document_xml(&out);
    assert_eq!(xml.matches("<w:tblPrEx>").count(), 1, "{xml}");
    assert!(xml.contains(r#"<w:tr><w:tblPrEx><w:tblW w:w="3000" w:type="dxa"/><w:jc w:val="center"/><w:tblInd w:w="240" w:type="dxa"/><w:tblLayout w:type="fixed"/></w:tblPrEx><w:trPr>"#), "{xml}");
    // Reading the output again gives the same exception on the first row only.
    let back = wordcraft_docx::read(&out).unwrap();
    let t = back.body.iter().find_map(|b| b.as_table()).unwrap();
    let x = t.rows[0].props.exceptions.as_ref().expect("first row keeps its exceptions");
    assert_eq!(x.width, Some(150.0));
    assert!(x.fixed);
    assert!(t.rows[1].props.exceptions.is_none());
}

#[test]
fn an_empty_row_exception_element_is_kept() {
    let body = format!(r#"<w:tbl><w:tblPr/><w:tblGrid><w:gridCol w:w="2000"/></w:tblGrid>{}</w:tbl><w:p/>"#, row("<w:tblPrEx/>", "A"));
    let d = wordcraft_docx::read(&docx(&body)).unwrap();
    let xml = document_xml(&wordcraft_docx::write(&d).unwrap());
    assert!(xml.contains("<w:tblPrEx></w:tblPrEx>") || xml.contains("<w:tblPrEx/>"), "{xml}");
}
