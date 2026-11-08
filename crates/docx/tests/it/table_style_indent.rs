//! The indent of a table style (`w:tblStyle/w:tblPr/w:tblInd`, ECMA-376 §17.4.50). pandoc's
//! "Table" style sets it to 0; dropped on save, Word for Mac moved every table of a manuscript
//! about 5.4 pt to the right (it places a table whose style chain sets no indent at the margin,
//! but one set to 0 with its first cell's text at the margin).

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

fn style_xml(bytes: &[u8], id: &str) -> String {
    let mut z = zip::ZipArchive::new(std::io::Cursor::new(bytes)).unwrap();
    let mut s = String::new();
    z.by_name("word/styles.xml").unwrap().read_to_string(&mut s).unwrap();
    let key = format!(r#"w:styleId="{id}""#);
    s.split("<w:style ").find(|x| x.contains(&key)).and_then(|x| x.split("</w:style>").next()).unwrap_or_default().to_string()
}

const BODY: &str = r#"<w:tbl><w:tblPr><w:tblStyle w:val="Indented"/><w:tblW w:w="0" w:type="auto"/></w:tblPr><w:tblGrid><w:gridCol w:w="2000"/></w:tblGrid><w:tr><w:tc><w:p><w:r><w:t>Cell</w:t></w:r></w:p></w:tc></w:tr></w:tbl><w:p/>"#;

#[test]
fn a_table_style_indent_of_zero_survives_a_save() {
    let styles = r#"<w:style w:type="table" w:default="1" w:styleId="Table"><w:name w:val="Table"/><w:tblPr><w:tblInd w:w="0" w:type="dxa"/><w:tblCellMar><w:left w:w="108" w:type="dxa"/><w:right w:w="108" w:type="dxa"/></w:tblCellMar></w:tblPr></w:style><w:style w:type="table" w:styleId="Indented"><w:name w:val="Indented"/><w:basedOn w:val="Table"/><w:tblPr><w:tblInd w:w="720" w:type="dxa"/></w:tblPr></w:style><w:style w:type="table" w:styleId="Plain"><w:name w:val="Plain"/><w:tblPr><w:tblCellMar><w:left w:w="108" w:type="dxa"/></w:tblCellMar></w:tblPr></w:style>"#;
    let d = wordcraft_docx::read(&docx(BODY, styles)).unwrap();
    let parts = |id: &str| d.styles.get(id).and_then(|s| s.table.clone()).unwrap_or_default();
    assert_eq!(parts("Table").indent, Some(0.0));
    assert_eq!(parts("Indented").indent, Some(36.0));
    assert_eq!(parts("Plain").indent, None, "no w:tblInd stays unset");

    let out = wordcraft_docx::write(&d).unwrap();
    let table = style_xml(&out, "Table");
    assert!(table.contains(r#"<w:tblInd w:w="0" w:type="dxa"/>"#), "{table}");
    let indented = style_xml(&out, "Indented");
    assert!(indented.contains(r#"<w:tblInd w:w="720" w:type="dxa"/>"#), "{indented}");
    let plain = style_xml(&out, "Plain");
    assert!(!plain.contains("w:tblInd"), "{plain}");
    // ECMA-376 order in w:tblPr: w:tblInd before w:tblCellMar.
    assert!(table.find("w:tblInd") < table.find("w:tblCellMar"), "{table}");
}

#[test]
fn a_table_style_indent_that_is_no_length_sets_none() {
    // `pct` and `auto` are no lengths for an indent; `nil` is none.
    let style = |ty: &str| {
        format!(
            r#"<w:style w:type="table" w:styleId="Indented"><w:name w:val="Indented"/><w:tblPr><w:tblInd w:w="720" w:type="{ty}"/></w:tblPr></w:style>"#
        )
    };
    for (ty, want) in [("pct", None), ("auto", None), ("nil", Some(0.0)), ("dxa", Some(36.0))] {
        let d = wordcraft_docx::read(&docx(BODY, &style(ty))).unwrap();
        let got = d.styles.get("Indented").and_then(|s| s.table.clone()).unwrap_or_default().indent;
        assert_eq!(got, want, "w:type={ty}");
    }
}
