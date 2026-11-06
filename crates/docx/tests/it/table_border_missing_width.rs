//! A table or cell border without `w:sz` (as pandoc writes its table header rule). Word for Mac
//! reads it as `w:sz="0"`: a hairline that takes no room. Read as ½ pt and saved as `w:sz="4"`,
//! it pushed every row below the header ½ pt down in Word. A paragraph border without `w:sz`
//! keeps the ½ pt default.

use std::io::{Read, Write};

fn docx(body: &str, styles: &str) -> Vec<u8> {
    let root = r#"<?xml version="1.0" encoding="UTF-8"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="word/document.xml"/></Relationships>"#;
    let doc_rels = r#"<?xml version="1.0" encoding="UTF-8"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/styles" Target="styles.xml"/></Relationships>"#;
    let doc = format!(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:body>{body}</w:body></w:document>"#
    );
    let styles = format!(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><w:styles xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">{styles}</w:styles>"#
    );
    let parts = [
        ("_rels/.rels", root.to_string()),
        ("word/document.xml", doc),
        ("word/_rels/document.xml.rels", doc_rels.to_string()),
        ("word/styles.xml", styles),
    ];
    let mut zw = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    for (name, xml) in &parts {
        zw.start_file(*name, zip::write::SimpleFileOptions::default()).unwrap();
        zw.write_all(xml.as_bytes()).unwrap();
    }
    zw.finish().unwrap().into_inner()
}

fn part(bytes: &[u8], name: &str) -> String {
    let mut z = zip::ZipArchive::new(std::io::Cursor::new(bytes)).unwrap();
    let mut s = String::new();
    z.by_name(name).unwrap().read_to_string(&mut s).unwrap();
    s
}

/// The opening tags of `<w:{side}` elements in `xml`.
fn sides(xml: &str, side: &str) -> Vec<String> {
    let tag = format!("<w:{side} ");
    xml.match_indices(&tag).map(|(i, _)| xml[i..].split('>').next().unwrap_or("").to_string()).collect()
}

#[test]
fn table_borders_without_a_size_are_width_zero_and_save_as_sz_0() {
    // The pandoc table style, and a table and a cell border with no w:sz.
    let styles = r#"<w:style w:type="table" w:default="1" w:styleId="Table"><w:name w:val="Table"/><w:tblPr><w:tblCellMar><w:left w:w="108" w:type="dxa"/><w:right w:w="108" w:type="dxa"/></w:tblCellMar></w:tblPr><w:tblStylePr w:type="firstRow"><w:tcPr><w:tcBorders><w:bottom w:val="single"/></w:tcBorders><w:vAlign w:val="bottom"/></w:tcPr></w:tblStylePr></w:style>"#;
    let body = r#"<w:tbl><w:tblPr><w:tblStyle w:val="Table"/><w:tblW w:w="5000" w:type="pct"/><w:tblBorders><w:top w:val="single"/></w:tblBorders><w:tblLook w:firstRow="1" w:val="0020"/></w:tblPr><w:tblGrid><w:gridCol w:w="2000"/></w:tblGrid><w:tr><w:tc><w:tcPr><w:tcBorders><w:left w:val="single"/></w:tcBorders></w:tcPr><w:p><w:r><w:t>Head</w:t></w:r></w:p></w:tc></w:tr><w:tr><w:tc><w:p><w:r><w:t>Body</w:t></w:r></w:p></w:tc></w:tr></w:tbl><w:p/>"#;
    let d = wordcraft_docx::read(&docx(body, styles)).unwrap();

    let ts = d.styles.get("Table").and_then(|s| s.table.as_ref()).expect("table style parts");
    let rule = ts.header_borders.and_then(|b| b.bottom).expect("the firstRow bottom border");
    assert_eq!(rule.width, 0.0, "{rule:?}");
    assert!(!rule.is_visible() && rule.is_drawn(), "a width-0 border takes no room but is drawn: {rule:?}");
    let t = d.body[0].as_table().unwrap();
    assert_eq!(t.props.borders.and_then(|b| b.top).map(|b| b.width), Some(0.0), "w:tblBorders");
    assert_eq!(t.rows[0].cells[0].props.borders.and_then(|b| b.left).map(|b| b.width), Some(0.0), "w:tcBorders");

    let out = wordcraft_docx::write(&d).unwrap();
    let styles_out = part(&out, "word/styles.xml");
    let table_style = styles_out
        .split("<w:style ")
        .find(|s| s.contains(r#"w:styleId="Table""#))
        .and_then(|s| s.split("</w:style>").next())
        .expect("the Table style is written");
    let bottoms = sides(table_style, "bottom");
    assert!(bottoms.iter().any(|b| b.contains(r#"w:val="single""#) && b.contains(r#"w:sz="0""#)), "{bottoms:?}");
    assert!(!table_style.contains(r#"w:sz="4""#), "no ½ pt border appears on save: {table_style}");
    let doc_out = part(&out, "word/document.xml");
    assert!(sides(&doc_out, "top").iter().any(|b| b.contains(r#"w:sz="0""#)), "{doc_out}");
    assert!(sides(&doc_out, "left").iter().any(|b| b.contains(r#"w:sz="0""#)), "{doc_out}");
}

#[test]
fn a_paragraph_border_without_a_size_keeps_half_a_point() {
    let body = r#"<w:p><w:pPr><w:pBdr><w:bottom w:val="single"/></w:pBdr></w:pPr><w:r><w:t>Ruled</w:t></w:r></w:p>"#;
    let d = wordcraft_docx::read(&docx(body, "")).unwrap();
    let b = d.body[0].as_para().unwrap().props.borders.and_then(|b| b.bottom).expect("the paragraph border");
    assert_eq!(b.width, 0.5);
    let doc_out = part(&wordcraft_docx::write(&d).unwrap(), "word/document.xml");
    assert!(sides(&doc_out, "bottom").iter().any(|b| b.contains(r#"w:sz="4""#)), "{doc_out}");
}
