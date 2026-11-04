//! A whole row's tracked insertion and deletion (`w:trPr/w:ins`, `w:trPr/w:del`, ECMA-376 Part 1
//! §17.13.5.17 and §17.13.5.13) survive opening and saving. Without the insertion, a
//! counterparty's added row saved back as an ordinary row: it no longer showed as their change,
//! and Reject All in Word kept it. Fixtures are hand-written from the spec.

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

const DATE: &str = "2026-10-20T09:00:00Z";

fn row(trpr: &str, text: &str, mark: &str) -> String {
    let content = match mark {
        "del" => format!(r#"<w:del w:id="9" w:author="Counterparty X" w:date="{DATE}"><w:r><w:delText>{text}</w:delText></w:r></w:del>"#),
        "ins" => format!(r#"<w:ins w:id="9" w:author="Counterparty X" w:date="{DATE}"><w:r><w:t>{text}</w:t></w:r></w:ins>"#),
        _ => format!("<w:r><w:t>{text}</w:t></w:r>"),
    };
    format!("<w:tr>{trpr}<w:tc><w:p>{content}</w:p></w:tc></w:tr>")
}

/// The `<w:tr>…</w:tr>` that holds `text`.
fn row_xml<'a>(xml: &'a str, text: &str) -> &'a str {
    let at = xml.find(text).unwrap();
    let start = xml[..at].rfind("<w:tr>").unwrap();
    let end = at + xml[at..].find("</w:tr>").unwrap();
    &xml[start..end]
}

#[test]
fn a_rows_tracked_insertion_and_deletion_survive_open_and_save() {
    let body = format!(
        r#"<w:tbl><w:tblPr/><w:tblGrid><w:gridCol w:w="4000"/></w:tblGrid>{}{}{}</w:tbl><w:p/>"#,
        row("", "Item", ""),
        row(&format!(r#"<w:trPr><w:del w:id="1" w:author="Counterparty X" w:date="{DATE}"/></w:trPr>"#), "Demurrage", "del"),
        row(&format!(r#"<w:trPr><w:cantSplit/><w:ins w:id="2" w:author="Counterparty X" w:date="{DATE}"/></w:trPr>"#), "Despatch", "ins"),
    );
    let body = body.replace(
        "</w:tbl>",
        &format!(
            "{}</w:tbl>",
            row(&format!(r#"<w:trPr><w:ins w:id="3" w:author="Counterparty X" w:date="{DATE}"/><w:del w:id="4" w:author="Reviewer Y" w:date="{DATE}"/></w:trPr>"#), "Laytime", "")
        ),
    );
    let d = wordcraft_docx::read(&docx(&body)).unwrap();
    let xml = document_xml(&wordcraft_docx::write(&d).unwrap());
    let deleted = row_xml(&xml, "Demurrage");
    assert!(deleted.contains("<w:trPr><w:del ") && deleted.contains(r#"w:author="Counterparty X""#), "{deleted}");
    // The row's mark comes after its properties (CT_TrPr: properties, then w:ins, w:del).
    let inserted = row_xml(&xml, "Despatch");
    assert!(inserted.contains("<w:trPr><w:cantSplit/><w:ins "), "{inserted}");
    assert!(!row_xml(&xml, "Item").contains("<w:trPr>"), "an ordinary row stays plain: {xml}");
    // A row inserted by one author and deleted by another keeps both, insertion first.
    let both = row_xml(&xml, "Laytime");
    let (i, d) = (both.find("<w:ins ").expect(both), both.find("<w:del ").expect(both));
    assert!(i < d && both.contains(r#"w:author="Reviewer Y""#), "{both}");
}
