//! What page borders are measured from (`w:offsetFrom` on `w:pgBorders`, ECMA-376 Part 1
//! §17.6.10): the page edge or the text, written back as read, the attribute's absence read as
//! its default (the text). A border made here is measured from the page edge, as the layout
//! draws it. Word places the box by it, so a save that turns one into the other moves the box.
//! Fixtures are hand-written from the spec.

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

#[test]
fn page_border_offset_is_written_back_as_read() {
    let sect = |offset: &str| {
        format!(
            r#"<w:p><w:pPr><w:sectPr><w:pgBorders{offset}><w:top w:val="single" w:sz="36" w:space="24" w:color="000000"/></w:pgBorders></w:sectPr></w:pPr></w:p>"#
        )
    };
    let body = [sect(r#" w:offsetFrom="page""#), sect(r#" w:offsetFrom="text""#), sect(""), r#"<w:p/><w:sectPr/>"#.to_string()].concat();
    let d = wordcraft_docx::read(&docx(&body)).unwrap();
    let got: Vec<(bool, bool)> = d.sections().iter().map(|(_, s)| (s.page_borders.is_some(), s.page_borders_from_text)).collect();
    assert_eq!(got, vec![(true, false), (true, true), (true, true), (false, false)]);
    let out = wordcraft_docx::write(&d).unwrap();
    let xml = document_xml(&out);
    assert_eq!(xml.matches(r#"<w:pgBorders w:offsetFrom="page">"#).count(), 1, "{xml}");
    assert_eq!(xml.matches(r#"<w:pgBorders w:offsetFrom="text">"#).count(), 2, "{xml}");
    let back = wordcraft_docx::read(&out).unwrap();
    assert_eq!(back.sections().iter().map(|(_, s)| (s.page_borders.is_some(), s.page_borders_from_text)).collect::<Vec<_>>(), got);
}

#[test]
fn a_new_page_border_is_measured_from_the_page_edge() {
    let mut d = wordcraft_docx::read(&docx("<w:p/>")).unwrap();
    d.last_section.page_borders = Some(Borders::box_(Border::single(1.0)));
    let xml = document_xml(&wordcraft_docx::write(&d).unwrap());
    assert!(xml.contains(r#"<w:pgBorders w:offsetFrom="page">"#), "{xml}");
}
