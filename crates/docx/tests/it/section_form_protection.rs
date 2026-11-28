//! Section form protection (`w:formProt` in `w:sectPr`, ECMA-376 Part 1 §17.6.6). Fixtures are
//! hand-written from the spec.

fn docx(body: &str) -> Vec<u8> {
    let root = r#"<?xml version="1.0" encoding="UTF-8"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="word/document.xml"/></Relationships>"#;
    let doc = format!(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:body>{body}</w:body></w:document>"#
    );
    let mut zw = craft_zip::Writer::new();
    for (name, bytes) in [("_rels/.rels", root.as_bytes()), ("word/document.xml", doc.as_bytes())] {
        zw.add(name, bytes, craft_zip::Compression::Deflated).unwrap();
    }
    zw.finish().unwrap()
}

fn document_xml(bytes: &[u8]) -> String {
    let mut z = craft_zip::Archive::open(bytes, craft_zip::Limits { max_entries: 10_000, max_total_bytes: 64 << 20 }).unwrap();

    String::from_utf8(z.read("word/document.xml", 16 << 20).unwrap().unwrap()).unwrap()
}

#[test]
fn form_protection_round_trips_in_schema_order() {
    // Three sections: protected, explicitly unprotected, and one that says nothing.
    let body = concat!(
        r#"<w:p><w:pPr><w:sectPr><w:formProt/><w:titlePg/></w:sectPr></w:pPr><w:r><w:t>A</w:t></w:r></w:p>"#,
        r#"<w:p><w:pPr><w:sectPr><w:formProt w:val="0"/></w:sectPr></w:pPr><w:r><w:t>B</w:t></w:r></w:p>"#,
        r#"<w:p><w:r><w:t>C</w:t></w:r></w:p><w:sectPr/>"#
    );
    let d = wordcraft_docx::read(&docx(body)).unwrap();
    let flags: Vec<Option<bool>> = d.sections().iter().map(|(_, s)| s.form_prot).collect();
    assert_eq!(flags, vec![Some(true), Some(false), None]);
    let out = wordcraft_docx::write(&d).unwrap();
    let xml = document_xml(&out);
    // Before w:titlePg in CT_SectPr's sequence; a section that never said it writes none.
    assert!(xml.contains(r#"<w:formProt/><w:titlePg/>"#), "{xml}");
    assert!(xml.contains(r#"<w:formProt w:val="0"/>"#), "{xml}");
    assert_eq!(xml.matches("w:formProt").count(), 2);
    let again: Vec<Option<bool>> = wordcraft_docx::read(&out).unwrap().sections().iter().map(|(_, s)| s.form_prot).collect();
    assert_eq!(again, flags);
}
