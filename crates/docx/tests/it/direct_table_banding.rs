//! Direct table band sizes survive repeated DOCX saves; invalid row-exception tags are not emitted.

use std::io::{Read, Write};

fn docx(body: &str) -> Vec<u8> {
    let root = r#"<?xml version="1.0" encoding="UTF-8"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="word/document.xml"/></Relationships>"#;
    let rels = r#"<?xml version="1.0" encoding="UTF-8"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/styles" Target="styles.xml"/></Relationships>"#;
    let doc = format!(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:body>{body}</w:body></w:document>"#
    );
    let styles = r#"<?xml version="1.0" encoding="UTF-8"?><w:styles xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:style w:type="table" w:styleId="Control"><w:name w:val="Control"/><w:tblPr><w:tblStyleRowBandSize w:val="6"/><w:tblStyleColBandSize w:val="7"/></w:tblPr></w:style></w:styles>"#;
    let mut zw = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    for (name, bytes) in [
        ("_rels/.rels", root.as_bytes()),
        ("word/_rels/document.xml.rels", rels.as_bytes()),
        ("word/document.xml", doc.as_bytes()),
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

fn table(row_band: &str, col_band: &str, exception: &str) -> String {
    format!(
        r#"<w:tbl><w:tblPr><w:tblStyle w:val="Control"/>{row_band}{col_band}<w:tblLook/></w:tblPr><w:tblGrid><w:gridCol w:w="2000"/></w:tblGrid><w:tr>{exception}<w:tc><w:tcPr/><w:p/></w:tc></w:tr></w:tbl><w:p/>"#
    )
}

#[test]
fn direct_table_band_sizes_survive_json_and_two_docx_saves_without_invalid_row_exceptions() {
    let exception = r#"<w:tblPrEx><w:tblStyleRowBandSize w:val="4"/><w:tblStyleColBandSize w:val="5"/></w:tblPrEx>"#;
    let input = docx(&table(r#"<w:tblStyleRowBandSize w:val="2"/>"#, r#"<w:tblStyleColBandSize w:val="3"/>"#, exception));
    let doc = wordcraft_docx::read(&input).unwrap();
    let native_json = serde_json::to_vec(&doc).unwrap();
    let native_json = String::from_utf8(native_json).unwrap();
    assert!(!native_json.contains("\"bandSize\":null"), "{native_json}");
    assert!(!native_json.contains("\"colBandSize\":null"), "{native_json}");
    let mut doc: wordcraft_doc::Document = serde_json::from_str(&native_json).unwrap();
    assert_eq!(doc.styles.table_style("Control").map(|s| (s.parts.band_size, s.parts.col_band_size)), Some((Some(6), Some(7))));
    let table = doc.body.iter().find_map(|b| b.as_table()).unwrap();
    let ex = table.rows[0].props.exceptions.as_ref().expect("read row exception");
    assert_eq!((ex.band_size, ex.col_band_size), (None, None));

    for _ in 0..2 {
        let tbl = doc.body.iter().find_map(|b| b.as_table()).unwrap();
        assert_eq!((tbl.props.band_size, tbl.props.col_band_size), (Some(2), Some(3)));
        let saved = wordcraft_docx::write(&doc).unwrap();
        let xml = document_xml(&saved);
        assert!(xml.contains(r#"<w:tblStyleRowBandSize w:val="2"/>"#), "{xml}");
        assert!(xml.contains(r#"<w:tblStyleColBandSize w:val="3"/>"#), "{xml}");
        let ex_xml = xml.split_once("<w:tblPrEx>").unwrap().1.split_once("</w:tblPrEx>").unwrap().0;
        assert!(!ex_xml.contains("tblStyleRowBandSize"), "row-exception band sizes are not valid CT_TblPrExBase children: {xml}");
        assert!(!ex_xml.contains("tblStyleColBandSize"), "row-exception band sizes are not valid CT_TblPrExBase children: {xml}");
        let tblpr = xml.split_once("<w:tblPr>").unwrap().1.split_once("</w:tblPr>").unwrap().0;
        assert!(tblpr.find("w:tblStyleColBandSize").unwrap() < tblpr.find("w:tblW").unwrap(), "{tblpr}");
        doc = wordcraft_docx::read(&saved).unwrap();
        assert_eq!(doc.styles.table_style("Control").map(|s| (s.parts.band_size, s.parts.col_band_size)), Some((Some(6), Some(7))));
        let table = doc.body.iter().find_map(|b| b.as_table()).unwrap();
        assert_eq!((table.props.band_size, table.props.col_band_size), (Some(2), Some(3)));
        let ex = table.rows[0].props.exceptions.as_ref().expect("row exception retained");
        assert_eq!((ex.band_size, ex.col_band_size), (None, None));
    }
}

#[test]
fn direct_band_sizes_preserve_zero_and_bound_invalid_or_oversized_values() {
    let input = docx(&table(
        r#"<w:tblStyleRowBandSize w:val="-00"/><w:tblStyleColBandSize w:val="4294967296"/>"#,
        "",
        r#"<w:tblPrEx><w:tblStyleRowBandSize w:val="bad"/><w:tblStyleColBandSize w:val="5000"/></w:tblPrEx>"#,
    ));
    let doc = wordcraft_docx::read(&input).unwrap();
    let tbl = doc.body.iter().find_map(|b| b.as_table()).unwrap();
    assert_eq!((tbl.props.band_size, tbl.props.col_band_size), (Some(0), Some(1000)));
    let ex = tbl.rows[0].props.exceptions.as_ref().expect("row exception retained");
    assert_eq!((ex.band_size, ex.col_band_size), (None, None));
    let saved = wordcraft_docx::write(&doc).unwrap();
    let xml = document_xml(&saved);
    assert!(xml.contains(r#"<w:tblStyleRowBandSize w:val="0"/>"#), "{xml}");
    assert!(xml.contains(r#"<w:tblStyleColBandSize w:val="1000"/>"#), "{xml}");
    assert!(!xml.contains("4294967296"), "{xml}");
    assert!(!xml.contains("val=\"bad\""), "{xml}");
}

#[test]
fn direct_band_sizes_accept_negative_zero_only() {
    let input = docx(&table(
        r#"<w:tblStyleRowBandSize w:val="-0"/><w:tblStyleColBandSize w:val="-00"/>"#,
        "",
        r#"<w:tblPrEx><w:tblStyleRowBandSize w:val="-1"/><w:tblStyleColBandSize w:val="-"/></w:tblPrEx>"#,
    ));
    let doc = wordcraft_docx::read(&input).unwrap();
    let tbl = doc.body.iter().find_map(|b| b.as_table()).unwrap();
    assert_eq!((tbl.props.band_size, tbl.props.col_band_size), (Some(0), Some(0)));
    let ex = tbl.rows[0].props.exceptions.as_ref().expect("row exception retained");
    assert_eq!((ex.band_size, ex.col_band_size), (None, None));
}

#[test]
fn direct_band_sizes_ignore_fractional_and_exponent_values() {
    let input = docx(&table(r#"<w:tblStyleRowBandSize w:val="1.5"/><w:tblStyleColBandSize w:val="1e2"/>"#, "", ""));
    let doc = wordcraft_docx::read(&input).unwrap();
    let tbl = doc.body.iter().find_map(|b| b.as_table()).unwrap();
    assert_eq!((tbl.props.band_size, tbl.props.col_band_size), (None, None));
}

#[test]
fn direct_band_sizes_trim_and_cap_integer_values() {
    let input = docx(&table(r#"<w:tblStyleRowBandSize w:val=" 2 "/><w:tblStyleColBandSize w:val=" 1001 "/>"#, "", ""));
    let doc = wordcraft_docx::read(&input).unwrap();
    let tbl = doc.body.iter().find_map(|b| b.as_table()).unwrap();
    assert_eq!((tbl.props.band_size, tbl.props.col_band_size), (Some(2), Some(1000)));
}

#[test]
fn absent_direct_band_sizes_stay_absent() {
    let input = docx(&table("", "", ""));
    let doc = wordcraft_docx::read(&input).unwrap();
    let saved = wordcraft_docx::write(&doc).unwrap();
    let xml = document_xml(&saved);
    assert!(!xml.contains("tblStyleRowBandSize"), "{xml}");
    assert!(!xml.contains("tblStyleColBandSize"), "{xml}");
}
