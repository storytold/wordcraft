//! Original partial-cell-margin fixture (ECMA-376 §17.4.69).
use std::io::{Cursor, Read, Write};

fn fixture() -> Vec<u8> {
    let document = r#"<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:body><w:tbl><w:tblPr><w:tblCellMar><w:top w:w="55" w:type="dxa"/><w:left w:w="45" w:type="dxa"/><w:bottom w:w="55" w:type="dxa"/><w:right w:w="55" w:type="dxa"/></w:tblCellMar></w:tblPr><w:tblGrid><w:gridCol w:w="4320"/></w:tblGrid><w:tr><w:tc><w:tcPr><w:tcMar><w:left w:w="0" w:type="dxa"/></w:tcMar></w:tcPr><w:p><w:r><w:t>Point</w:t></w:r></w:p></w:tc></w:tr></w:tbl></w:body></w:document>"#;
    let mut zip = zip::ZipWriter::new(Cursor::new(Vec::new()));
    zip.start_file("word/document.xml", zip::write::SimpleFileOptions::default()).unwrap();
    zip.write_all(document.as_bytes()).unwrap();
    zip.finish().unwrap().into_inner()
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
    let mut zip = zip::ZipArchive::new(Cursor::new(bytes)).unwrap();
    let mut xml = String::new();
    zip.by_name("word/document.xml").unwrap().read_to_string(&mut xml).unwrap();
    let margins = xml.split("<w:tcMar>").nth(1).unwrap().split("</w:tcMar>").next().unwrap();
    assert!(margins.contains("<w:left w:w=\"0\""));
    assert!(!margins.contains("<w:top"));
    assert!(!margins.contains("<w:bottom"));
    assert!(!margins.contains("<w:right"));
}

/// A package with this body and, if given, this styles part.
fn package(body: &str, styles: Option<&str>) -> Vec<u8> {
    const W: &str = "http://schemas.openxmlformats.org/wordprocessingml/2006/main";
    let mut zip = zip::ZipWriter::new(Cursor::new(Vec::new()));
    let mut add = |name: &str, xml: &str| {
        zip.start_file(name, zip::write::SimpleFileOptions::default()).unwrap();
        zip.write_all(xml.as_bytes()).unwrap();
    };
    add("word/document.xml", &format!(r#"<w:document xmlns:w="{W}"><w:body>{body}</w:body></w:document>"#));
    if let Some(styles) = styles {
        add("word/styles.xml", &format!(r#"<w:styles xmlns:w="{W}">{styles}</w:styles>"#));
        add(
            "word/_rels/document.xml.rels",
            r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/styles" Target="styles.xml"/></Relationships>"#,
        );
    }
    zip.finish().unwrap().into_inner()
}

/// Without a default table style (no styles part, or none marked `w:default`) Word gives a table
/// no cell margins: a cell that sets only its right margin has none on its other sides.
#[test]
fn no_default_table_style_means_no_cell_margins() {
    let body = r#"<w:tbl><w:tblPr><w:tblW w:w="6000" w:type="dxa"/></w:tblPr><w:tblGrid><w:gridCol w:w="3000"/></w:tblGrid><w:tr><w:tc><w:tcPr><w:tcMar><w:right w:w="108" w:type="dxa"/></w:tcMar></w:tcPr><w:p/></w:tc></w:tr></w:tbl>"#;
    let normal_table = r#"<w:style w:type="table" w:default="1" w:styleId="TableNormal"><w:name w:val="Normal Table"/><w:tblPr><w:tblCellMar><w:left w:w="108" w:type="dxa"/><w:right w:w="108" w:type="dxa"/></w:tblCellMar></w:tblPr></w:style>"#;
    let margins = |styles: Option<&str>| {
        let doc = wordcraft_docx::read(&package(body, styles)).unwrap();
        let table = doc.body[0].as_table().unwrap();
        (table.props.cell_margins, table.rows[0].cells[0].props.resolved_margins(table.props.cell_margins.unwrap_or([0.0, 5.4, 0.0, 5.4])))
    };
    assert_eq!(margins(None), (Some([0.0; 4]), [0.0, 0.0, 0.0, 5.4]));
    assert_eq!(margins(Some(r#"<w:docDefaults/>"#)), (Some([0.0; 4]), [0.0, 0.0, 0.0, 5.4]));
    // With Word's usual Normal Table the table keeps no margins of its own (the defaults apply).
    assert_eq!(margins(Some(normal_table)).0, None);
    // A style defined twice is its last definition: a default one redefined as not default leaves none.
    let redefined = format!(r#"{normal_table}<w:style w:type="table" w:styleId="TableNormal"><w:name w:val="Normal Table"/></w:style>"#);
    assert_eq!(margins(Some(&redefined)).0, Some([0.0; 4]));
    // A custom default table style marked `on` is a default too, also after saving and reopening.
    let plain = r#"<w:style w:type="table" w:default="on" w:customStyle="1" w:styleId="Plain"><w:name w:val="Plain"/></w:style>"#;
    let doc = wordcraft_docx::read(&package(body, Some(plain))).unwrap();
    assert_eq!(doc.body[0].as_table().unwrap().props.cell_margins, None);
    let reopened = wordcraft_docx::read(&wordcraft_docx::write(&doc).unwrap()).unwrap();
    assert_eq!(reopened.body[0].as_table().unwrap().props.cell_margins, None);
    // A table whose own style gives margins keeps them.
    let styled = body.replace("<w:tblPr>", r#"<w:tblPr><w:tblStyle w:val="Boxed"/>"#);
    let boxed = r#"<w:style w:type="table" w:styleId="Boxed"><w:name w:val="Boxed"/><w:tblPr><w:tblCellMar><w:left w:w="144" w:type="dxa"/></w:tblCellMar></w:tblPr></w:style>"#;
    let doc = wordcraft_docx::read(&package(&styled, Some(boxed))).unwrap();
    assert_eq!(doc.body[0].as_table().unwrap().props.cell_margins, None);
    // The zeros survive saving, which writes a default Normal Table.
    let doc = wordcraft_docx::read(&package(body, None)).unwrap();
    let bytes = wordcraft_docx::write(&doc).unwrap();
    let mut xml = String::new();
    zip::ZipArchive::new(Cursor::new(bytes.clone())).unwrap().by_name("word/document.xml").unwrap().read_to_string(&mut xml).unwrap();
    assert!(xml.contains("<w:tblCellMar>"), "{xml}");
    let reopened = wordcraft_docx::read(&bytes).unwrap();
    assert_eq!(reopened.body[0].as_table().unwrap().props.cell_margins, Some([0.0; 4]));
}
