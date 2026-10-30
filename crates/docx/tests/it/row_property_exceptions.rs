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

fn docx_with_styles(body: &str, styles: &str) -> Vec<u8> {
    let root = r#"<?xml version="1.0" encoding="UTF-8"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="word/document.xml"/></Relationships>"#;
    let rels = r#"<?xml version="1.0" encoding="UTF-8"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/styles" Target="styles.xml"/></Relationships>"#;
    let doc = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?><w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:body>{body}</w:body></w:document>"#
    );
    let mut zw = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    for (name, bytes) in [
        ("_rels/.rels", root.as_bytes()),
        ("word/document.xml", doc.as_bytes()),
        ("word/_rels/document.xml.rels", rels.as_bytes()),
        ("word/styles.xml", styles.as_bytes()),
    ] {
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

#[test]
fn table_cell_spacing_survives_table_and_row_exception_roundtrips() {
    let ex = r#"<w:tblPrEx><w:tblCellSpacing w:w="120" w:type="dxa"/></w:tblPrEx>"#;
    let body = format!(
        r#"<w:tbl><w:tblPr><w:tblCellSpacing w:w="120" w:type="dxa"/></w:tblPr><w:tblGrid><w:gridCol w:w="2000"/></w:tblGrid>{}</w:tbl><w:p/>"#,
        row(ex, "A")
    );
    let first = wordcraft_docx::read(&docx(&body)).unwrap();
    let table = first.body.iter().find_map(|b| b.as_table()).unwrap();
    assert_eq!(table.props.cell_spacing, Some(wordcraft_doc::props::CellSpacing { width: 120, kind: wordcraft_doc::props::CellSpacingKind::Dxa }));
    assert_eq!(table.rows[0].props.exceptions.as_ref().unwrap().cell_spacing, table.props.cell_spacing);
    let json = serde_json::to_string(&first).unwrap();
    assert!(json.contains("cellSpacing"));
    let mut bytes = wordcraft_docx::write(&first).unwrap();
    for _ in 0..2 {
        let xml = document_xml(&bytes);
        assert_eq!(xml.matches(r#"<w:tblCellSpacing w:w="120" w:type="dxa"/>"#).count(), 2, "{xml}");
        bytes = wordcraft_docx::write(&wordcraft_docx::read(&bytes).unwrap()).unwrap();
    }
}

#[test]
fn table_cell_spacing_preserves_supported_types_and_bounds_hostile_widths() {
    use wordcraft_doc::props::{CellSpacing, CellSpacingKind};

    let make_table = |spacing: &str| {
        format!(r#"<w:tbl><w:tblPr>{spacing}</w:tblPr><w:tblGrid><w:gridCol w:w="2000"/></w:tblGrid><w:tr><w:tc><w:p/></w:tc></w:tr></w:tbl>"#)
    };
    let cases = [
        (r#"<w:tblCellSpacing w:w="0" w:type="dxa"/>"#, Some(CellSpacing { width: 0, kind: CellSpacingKind::Dxa })),
        (r#"<w:tblCellSpacing w:w="240" w:type="pct"/>"#, Some(CellSpacing { width: 240, kind: CellSpacingKind::Pct })),
        (r#"<w:tblCellSpacing w:w="1" w:type="auto"/>"#, Some(CellSpacing { width: 1, kind: CellSpacingKind::Auto })),
        (r#"<w:tblCellSpacing w:w="2" w:type="nil"/>"#, Some(CellSpacing { width: 2, kind: CellSpacingKind::Nil })),
        (r#"<w:tblCellSpacing w:w="31681" w:type="dxa"/>"#, Some(CellSpacing { width: 31680, kind: CellSpacingKind::Dxa })),
        (r#"<w:tblCellSpacing w:w="999999" w:type="pct"/>"#, Some(CellSpacing { width: 50000, kind: CellSpacingKind::Pct })),
        (r#"<w:tblCellSpacing w:w="-1" w:type="dxa"/>"#, None),
        (r#"<w:tblCellSpacing w:w="oops" w:type="dxa"/>"#, None),
        (r#"<w:tblCellSpacing w:w="12" w:type="unsupported"/>"#, None),
        ("", None),
    ];
    for (spacing, expected) in cases {
        let body = format!("{}<w:p/>", make_table(spacing));
        let doc = wordcraft_docx::read(&docx(&body)).unwrap();
        let table = doc.body.iter().find_map(|b| b.as_table()).unwrap();
        assert_eq!(table.props.cell_spacing, expected, "{spacing}");
    }
}

#[test]
fn paragraph_suppress_overlap_preserves_style_and_framed_values_across_two_saves() {
    let styles = r#"<w:styles xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:style w:type="paragraph" w:styleId="NoOverlap"><w:name w:val="NoOverlap"/><w:pPr><w:suppressOverlap w:val="0"/></w:pPr></w:style></w:styles>"#;
    let body = r#"<w:p><w:pPr><w:pStyle w:val="NoOverlap"/><w:framePr w:wrap="around" w:vAnchor="text" w:hAnchor="margin" w:xAlign="center" w:y="1"/><w:suppressOverlap w:val="0"/></w:pPr></w:p><w:p><w:pPr><w:suppressOverlap/></w:pPr></w:p><w:p/>"#;
    let mut bytes = wordcraft_docx::write(&wordcraft_docx::read(&docx_with_styles(body, styles)).unwrap()).unwrap();
    for _ in 0..2 {
        let d = wordcraft_docx::read(&bytes).unwrap();
        assert_eq!(d.body[0].as_para().unwrap().props.suppress_overlap, Some(false));
        assert!(d.body[0].as_para().unwrap().props.frame.is_some());
        assert_eq!(d.body[1].as_para().unwrap().props.suppress_overlap, Some(true));
        assert_eq!(d.body[2].as_para().unwrap().props.suppress_overlap, None);
        assert_eq!(d.styles.get("NoOverlap").unwrap().para.suppress_overlap, Some(false));
        assert!(serde_json::to_string(&d).unwrap().contains("suppressOverlap"));
        bytes = wordcraft_docx::write(&d).unwrap();
    }
}
