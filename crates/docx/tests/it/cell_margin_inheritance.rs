//! Original partial-cell-margin fixture (ECMA-376 §17.4.69).

fn fixture() -> Vec<u8> {
    let document = r#"<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:body><w:tbl><w:tblPr><w:tblCellMar><w:top w:w="55" w:type="dxa"/><w:left w:w="45" w:type="dxa"/><w:bottom w:w="55" w:type="dxa"/><w:right w:w="55" w:type="dxa"/></w:tblCellMar></w:tblPr><w:tblGrid><w:gridCol w:w="4320"/></w:tblGrid><w:tr><w:tc><w:tcPr><w:tcMar><w:left w:w="0" w:type="dxa"/></w:tcMar></w:tcPr><w:p><w:r><w:t>Point</w:t></w:r></w:p></w:tc></w:tr></w:tbl></w:body></w:document>"#;
    let mut zip = craft_zip::Writer::new();
    zip.add("word/document.xml", document.as_bytes(), craft_zip::Compression::Deflated).unwrap();
    zip.finish().unwrap()
}

#[test]
fn partial_margin_stays_partial() {
    let doc = wordcraft_docx::read(&fixture()).unwrap();
    let table = doc.body[0].as_table().unwrap();
    assert_eq!(table.rows[0].cells[0].props.margins, None, "a left-only margin is not a complete override");
    assert_eq!(table.rows[0].cells[0].props.resolved_margins([6.0, 7.0, 8.0, 9.0]), [6.0, 0.0, 8.0, 9.0]);
    let bytes = wordcraft_docx::write(&doc).unwrap();
    let reopened = wordcraft_docx::read(&bytes).unwrap();
    let cell = &reopened.body[0].as_table().unwrap().rows[0].cells[0];
    assert_eq!(cell.props.resolved_margins([6.0, 7.0, 8.0, 9.0]), [6.0, 0.0, 8.0, 9.0]);
    let mut zip = craft_zip::Archive::open(&bytes, craft_zip::Limits { max_entries: 10_000, max_total_bytes: 64 << 20 }).unwrap();
    let xml = String::from_utf8(zip.read("word/document.xml", 16 << 20).unwrap().unwrap()).unwrap();
    let margins = xml.split("<w:tcMar>").nth(1).unwrap().split("</w:tcMar>").next().unwrap();
    assert!(margins.contains("<w:left w:w=\"0\""));
    assert!(!margins.contains("<w:top"));
    assert!(!margins.contains("<w:bottom"));
    assert!(!margins.contains("<w:right"));
}
