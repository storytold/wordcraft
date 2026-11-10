//! A `HYPERLINK` complex field with no result, as LibreOffice writes a link to an anchor in another file.
use std::io::Write;

use wordcraft_doc::{Path, StoryRef};

fn package(body: &str) -> Vec<u8> {
    let xml = format!(r#"<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:body>{body}</w:body></w:document>"#);
    let mut zip = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    for (name, value) in [
        ("word/document.xml", xml.as_str()),
        (
            "_rels/.rels",
            r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="main" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="word/document.xml"/></Relationships>"#,
        ),
    ] {
        zip.start_file(name, zip::write::SimpleFileOptions::default()).unwrap();
        zip.write_all(value.as_bytes()).unwrap();
    }
    zip.finish().unwrap().into_inner()
}

fn field(code: &str, result: &str) -> String {
    let result = if result.is_empty() { String::new() } else { format!("<w:r><w:t>{result}</w:t></w:r>") };
    format!(
        r#"<w:r><w:fldChar w:fldCharType="begin"/></w:r><w:r><w:instrText xml:space="preserve">{code}</w:instrText></w:r><w:r><w:fldChar w:fldCharType="separate"/></w:r>{result}<w:r><w:fldChar w:fldCharType="end"/></w:r>"#
    )
}

const CODE: &str = r#" HYPERLINK "http://example.org/ro.owl" \l "derives_from""#;

#[test]
fn an_empty_hyperlink_field_stays_a_field_through_saves() {
    let body = format!(r#"<w:p><w:r><w:t>Use </w:t></w:r>{}<w:r><w:t>derives_from</w:t></w:r></w:p>"#, field(CODE, ""));
    let mut doc = wordcraft_docx::read(&package(&body)).unwrap();
    for _ in 0..3 {
        let ranges = doc.field_ranges(StoryRef::Body);
        assert_eq!(ranges.len(), 1);
        assert_eq!(ranges[0].instr, CODE.trim());
        assert_eq!(ranges[0].start.path, Path::top(0));
        assert_eq!(ranges[0].start.path, ranges[0].end.path);
        doc = wordcraft_docx::read(&wordcraft_docx::write(&doc).unwrap()).unwrap();
    }
}

#[test]
fn a_hyperlink_field_with_a_result_is_still_a_link_on_its_text() {
    let body = format!(r#"<w:p>{}</w:p>"#, field(r#" HYPERLINK "http://example.org/" "#, "go"));
    let doc = wordcraft_docx::read(&wordcraft_docx::write(&wordcraft_docx::read(&package(&body)).unwrap()).unwrap()).unwrap();
    assert!(doc.field_ranges(StoryRef::Body).is_empty());
    let para = doc.body[0].as_para().unwrap();
    assert_eq!(para.plain_text(), "go");
    assert_eq!(para.props_of_char(0).link.as_deref(), Some("http://example.org/"));
}
