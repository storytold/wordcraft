//! Section printer, note and chapter-number metadata survives DOCX and native JSON saves.

fn docx(body: &str) -> Vec<u8> {
    let root = r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="word/document.xml"/></Relationships>"#;
    let doc = format!(
        r#"<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main" xmlns:w15="http://schemas.microsoft.com/office/word/2012/wordml" xmlns:mc="http://schemas.openxmlformats.org/markup-compatibility/2006" mc:Ignorable="w15"><w:body>{body}</w:body></w:document>"#
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
fn section_metadata_survives_native_json_and_two_docx_save_reopen_cycles() {
    let input = docx(
        r#"<w:p><w:pPr><w:sectPr><w:paperSrc w:first=" +261 " w:other=" 260 "/><w:noEndnote/><w15:footnoteColumns w:val="2"/><w:pgNumType w:fmt="decimal" w:chapStyle="1" w:chapSep="hyphen"/></w:sectPr></w:pPr><w:r><w:t>First section</w:t></w:r></w:p><w:p><w:r><w:t>Second section</w:t></w:r></w:p><w:sectPr><w:paperSrc w:first="12" w:other="34"/><w:noEndnote w:val="0"/><w15:footnoteColumns w:val="3"/><w:pgNumType w:fmt="upperRoman" w:chapStyle="10" w:chapSep="colon"/></w:sectPr>"#,
    );
    let mut document = wordcraft_docx::read(&input).unwrap();
    let json = serde_json::to_vec(&document).unwrap();
    document = serde_json::from_slice(&json).unwrap();
    for cycle in 0..2 {
        let bytes = wordcraft_docx::write(&document).unwrap();
        let xml = document_xml(&bytes);
        assert!(xml.contains(r#"<w:paperSrc w:first="261" w:other="260"/>"#), "cycle {cycle}: {xml}");
        assert!(xml.contains(r#"<w:paperSrc w:first="12" w:other="34"/>"#), "cycle {cycle}: {xml}");
        assert!(xml.contains("<w:noEndnote/>"), "cycle {cycle}: {xml}");
        assert!(xml.contains(r#"<w:noEndnote w:val="0"/>"#), "cycle {cycle}: {xml}");
        assert!(xml.contains(r#"<w15:footnoteColumns w:val="2"/>"#), "cycle {cycle}: {xml}");
        assert!(xml.contains(r#"<w15:footnoteColumns w:val="3"/>"#), "cycle {cycle}: {xml}");
        assert!(xml.contains(r#"w:chapStyle="1" w:chapSep="hyphen""#), "cycle {cycle}: {xml}");
        assert!(xml.contains(r#"w:chapStyle="10" w:chapSep="colon""#), "cycle {cycle}: {xml}");
        assert!(xml.contains("xmlns:w15=") && xml.contains("mc:Ignorable=\"w14 w15 wp14\""), "cycle {cycle}: {xml}");
        document = wordcraft_docx::read(&bytes).unwrap();
        let json = serde_json::to_vec(&document).unwrap();
        document = serde_json::from_slice(&json).unwrap();
    }
}

#[test]
fn invalid_section_metadata_is_ignored_and_missing_values_stay_missing() {
    let bytes = docx(
        r#"<w:p><w:r><w:t>Text</w:t></w:r></w:p><w:sectPr><w:paperSrc w:first="65536" w:other="-1"/><w15:footnoteColumns w:val="0"/><w:pgNumType w:chapStyle="256" w:chapSep="invalid"/></w:sectPr>"#,
    );
    let mut d = wordcraft_docx::read(&bytes).unwrap();
    let xml = document_xml(&wordcraft_docx::write(&d).unwrap());
    assert!(!xml.contains("paperSrc"), "{xml}");
    assert!(!xml.contains("footnoteColumns"), "{xml}");
    assert!(!xml.contains("chapStyle") && !xml.contains("chapSep"), "{xml}");
    assert!(!xml.contains("noEndnote"), "{xml}");

    d.last_section.footnote_columns = Some(0);
    let xml = document_xml(&wordcraft_docx::write(&d).unwrap());
    assert!(!xml.contains("footnoteColumns"), "{xml}");
}
