//! A table's space between cells (`w:tblCellSpacing`, ECMA-376 Part 1 §17.4.43/§17.4.44/§17.4.45)
//! on the table, a row's exceptions and the row itself, and a row's own alignment (`w:jc` in
//! `w:trPr`, §17.4.80): written back as read, in the schema's places. Word lays a table out by
//! them, so a save that drops them moves its cells. Fixtures are hand-written from the spec.

use std::io::{Read, Write};

use wordcraft_doc::props::Align;

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

fn row(prex: &str, trpr: &str, text: &str) -> String {
    format!(
        r#"<w:tr>{prex}<w:trPr>{trpr}</w:trPr><w:tc><w:tcPr><w:tcW w:w="2500" w:type="pct"/></w:tcPr><w:p><w:r><w:t>{text}</w:t></w:r></w:p></w:tc></w:tr>"#
    )
}

#[test]
fn cell_spacing_and_row_alignment_are_written_back_as_read() {
    let body = format!(
        r#"<w:tbl><w:tblPr><w:tblW w:w="5000" w:type="pct"/><w:jc w:val="center"/><w:tblCellSpacing w:w="15" w:type="dxa"/><w:tblInd w:w="0" w:type="dxa"/></w:tblPr><w:tblGrid><w:gridCol w:w="4000"/></w:tblGrid>{}{}{}{}</w:tbl><w:p/>"#,
        row("", r#"<w:tblHeader/><w:tblCellSpacing w:w="15" w:type="dxa"/><w:jc w:val="center"/>"#, "Head"),
        row(r#"<w:tblPrEx><w:tblCellSpacing w:w="40" w:type="dxa"/></w:tblPrEx>"#, r#"<w:jc w:val="right"/>"#, "Right"),
        row("", r#"<w:tblCellSpacing w:w="0" w:type="nil"/>"#, "Nil"),
        row("", "", "Plain")
    );
    let d = wordcraft_docx::read(&docx(&body)).unwrap();
    let t = d.body.first().and_then(|b| b.as_table()).unwrap();
    assert_eq!(t.props.cell_spacing, Some(0.75));
    let rows: Vec<(Option<f32>, Option<Align>, Option<f32>)> =
        t.rows.iter().map(|r| (r.props.cell_spacing, r.props.align, r.props.exceptions.as_ref().and_then(|e| e.cell_spacing))).collect();
    assert_eq!(rows, vec![(Some(0.75), Some(Align::Center), None), (None, Some(Align::Right), Some(2.0)), (None, None, None), (None, None, None)]);
    let out = wordcraft_docx::write(&d).unwrap();
    let xml = document_xml(&out);
    assert!(xml.contains(r#"<w:jc w:val="center"/><w:tblCellSpacing w:w="15" w:type="dxa"/><w:tblInd"#), "table: {xml}");
    assert!(xml.contains(r#"<w:tblHeader/><w:tblCellSpacing w:w="15" w:type="dxa"/><w:jc w:val="center"/></w:trPr>"#), "header row: {xml}");
    assert!(xml.contains(r#"<w:tblPrEx><w:tblCellSpacing w:w="40" w:type="dxa"/></w:tblPrEx>"#), "exceptions: {xml}");
    assert!(xml.contains(r#"<w:trPr><w:jc w:val="right"/></w:trPr>"#), "right row: {xml}");
    assert_eq!(xml.matches("w:tblCellSpacing").count(), 3, "nil and none write nothing: {xml}");
    let back = wordcraft_docx::read(&out).unwrap();
    let b = back.body.first().and_then(|b| b.as_table()).unwrap();
    assert_eq!(b.props.cell_spacing, t.props.cell_spacing);
    assert_eq!(b.rows.iter().map(|r| r.props.clone()).collect::<Vec<_>>(), t.rows.iter().map(|r| r.props.clone()).collect::<Vec<_>>());
}
