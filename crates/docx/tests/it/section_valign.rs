//! Section vertical alignment (`w:vAlign` in `w:sectPr`, ECMA-376 Part 1 §17.6.23, values from
//! ST_VerticalJc). Fixtures are hand-written from the spec.

use std::io::{Read, Write};

use wordcraft_doc::section::PageVAlign;

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

/// #1320: "both" (Justified) reads as Justify and writes back as "both"; the others keep theirs.
#[test]
fn section_vertical_alignment_round_trips_with_justified() {
    let sect = |v: &str| format!(r#"<w:sectPr><w:vAlign w:val="{v}"/></w:sectPr>"#);
    let body = format!(
        r#"<w:p><w:pPr>{}</w:pPr><w:r><w:t>A</w:t></w:r></w:p><w:p><w:pPr>{}</w:pPr><w:r><w:t>B</w:t></w:r></w:p><w:p><w:pPr>{}</w:pPr><w:r><w:t>C</w:t></w:r></w:p><w:p><w:pPr>{}</w:pPr><w:r><w:t>D</w:t></w:r></w:p><w:p><w:r><w:t>E</w:t></w:r></w:p>{}"#,
        sect("both"),
        sect("center"),
        sect("bottom"),
        sect("top"),
        sect("sideways")
    );
    let d = wordcraft_docx::read(&docx(&body)).unwrap();
    let got: Vec<PageVAlign> = d.sections().iter().map(|(_, s)| s.valign).collect();
    let want = vec![PageVAlign::Justify, PageVAlign::Center, PageVAlign::Bottom, PageVAlign::Top, PageVAlign::Top];
    assert_eq!(got, want);
    let out = wordcraft_docx::write(&d).unwrap();
    let xml = document_xml(&out);
    assert_eq!(xml.matches(r#"<w:vAlign w:val="both"/>"#).count(), 1, "{xml}");
    assert_eq!(xml.matches(r#"<w:vAlign w:val="center"/>"#).count(), 1, "{xml}");
    assert_eq!(xml.matches(r#"<w:vAlign w:val="bottom"/>"#).count(), 1, "{xml}");
    assert_eq!(xml.matches("w:vAlign").count(), 3, "top writes nothing: {xml}");
    let back = wordcraft_docx::read(&out).unwrap();
    assert_eq!(back.sections().iter().map(|(_, s)| s.valign).collect::<Vec<_>>(), want);
}
