//! A table style's own indent (`w:tblPr/w:tblInd` in a `w:style w:type="table"`) survives a save
//! (#3541). Original synthetic packages.

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
        ("word/_rels/document.xml.rels", doc_rels.to_string()),
        ("word/document.xml", doc),
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

/// The `w:style` element with id `id` in a written `styles.xml`.
fn style<'a>(styles: &'a str, id: &str) -> &'a str {
    let at = styles.find(&format!(r#"w:styleId="{id}""#)).unwrap_or_else(|| panic!("no style {id}: {styles}"));
    let start = styles[..at].rfind("<w:style ").unwrap();
    let end = at + styles[at..].find("</w:style>").unwrap() + "</w:style>".len();
    &styles[start..end]
}

/// A one-cell table in style `id`, with no indent of its own.
fn table(id: &str) -> String {
    format!(
        r#"<w:tbl><w:tblPr><w:tblStyle w:val="{id}"/><w:tblW w:w="4000" w:type="dxa"/></w:tblPr><w:tblGrid><w:gridCol w:w="4000"/></w:tblGrid><w:tr><w:tc><w:p><w:r><w:t>Cell</w:t></w:r></w:p></w:tc></w:tr></w:tbl><w:p/>"#
    )
}

#[test]
fn a_table_style_keeps_its_indent_through_two_saves() {
    // The style of the report: nothing in its table properties but the indent.
    let styles = r#"<w:style w:type="table" w:styleId="G"><w:name w:val="G"/><w:tblPr><w:tblInd w:w="108" w:type="dxa"/></w:tblPr></w:style>"#;
    let once = wordcraft_docx::write(&wordcraft_docx::read(&docx(&table("G"), styles)).unwrap()).unwrap();
    let written = part(&once, "word/styles.xml");
    assert!(style(&written, "G").contains(r#"<w:tblInd w:w="108" w:type="dxa"/>"#), "{}", style(&written, "G"));
    let twice = wordcraft_docx::write(&wordcraft_docx::read(&once).unwrap()).unwrap();
    let again = part(&twice, "word/styles.xml");
    assert!(style(&again, "G").contains(r#"<w:tblInd w:w="108" w:type="dxa"/>"#), "{}", style(&again, "G"));
    // The table itself had no indent, and gains none: it still takes the style's.
    let body = part(&twice, "word/document.xml");
    assert!(!body.contains("w:tblInd"), "{body}");
}

#[test]
fn a_table_style_indent_is_written_in_schema_order_and_not_inherited_into_the_file() {
    // A hanging indent beside band sizes and borders, and a style based on it that sets none.
    let styles = concat!(
        r#"<w:style w:type="table" w:styleId="Hanging"><w:name w:val="Hanging"/><w:tblPr>"#,
        r#"<w:tblStyleRowBandSize w:val="1"/><w:tblInd w:w="-108" w:type="dxa"/>"#,
        r#"<w:tblBorders><w:top w:val="single" w:sz="4" w:space="0" w:color="000000"/></w:tblBorders>"#,
        r#"</w:tblPr></w:style>"#,
        r#"<w:style w:type="table" w:styleId="Child"><w:name w:val="Child"/><w:basedOn w:val="Hanging"/>"#,
        r#"<w:tblPr><w:tblStyleColBandSize w:val="1"/></w:tblPr></w:style>"#,
    );
    let body = format!("{}{}", table("Hanging"), table("Child"));
    let out = wordcraft_docx::write(&wordcraft_docx::read(&docx(&body, styles)).unwrap()).unwrap();
    let written = part(&out, "word/styles.xml");
    let hanging = style(&written, "Hanging");
    let (band, ind, borders) = (
        hanging.find("<w:tblStyleRowBandSize").unwrap_or(usize::MAX),
        hanging.find(r#"<w:tblInd w:w="-108" w:type="dxa"/>"#).unwrap_or(usize::MAX),
        hanging.find("<w:tblBorders").unwrap_or(usize::MAX),
    );
    assert!(ind != usize::MAX && band < ind && ind < borders, "{hanging}");
    // The based-on style keeps only what it set itself.
    let child = style(&written, "Child");
    assert!(!child.contains("w:tblInd"), "{child}");
}
