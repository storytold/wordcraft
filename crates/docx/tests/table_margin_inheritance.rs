//! Original table-wide margin controls. Omitted sides inherit; explicit zero overrides.
use std::io::{Cursor, Read, Write};

fn fixture(margins: &str) -> Vec<u8> {
    let document = format!(
        r#"<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:body><w:tbl><w:tblPr><w:tblCellMar>{margins}</w:tblCellMar></w:tblPr><w:tblGrid><w:gridCol w:w="720"/></w:tblGrid><w:tr><w:tc><w:p><w:r><w:t>Original margin control</w:t></w:r></w:p></w:tc></w:tr></w:tbl></w:body></w:document>"#
    );
    let mut zip = zip::ZipWriter::new(Cursor::new(Vec::new()));
    zip.start_file("word/document.xml", zip::write::SimpleFileOptions::default()).unwrap();
    zip.write_all(document.as_bytes()).unwrap();
    zip.finish().unwrap().into_inner()
}

#[test]
fn table_margin_sides_survive() {
    for sides in ["", r#"<w:right w:w="0" w:type="dxa"/>"#, r#"<w:end w:w="0" w:type="dxa"/>"#] {
        let expected = [None, None, None, (!sides.is_empty()).then_some(0.0)];
        let mut doc = wordcraft_docx::read(&fixture(sides)).unwrap();
        for _ in 0..3 {
            let props = &doc.body[0].as_table().unwrap().props;
            assert_eq!(props.cell_margins, None, "omitted sides are not zero overrides");
            assert_eq!(props.partial_cell_margins, Some(expected));
            let bytes = wordcraft_docx::write(&doc).unwrap();
            let mut zip = zip::ZipArchive::new(Cursor::new(&bytes)).unwrap();
            let mut xml = String::new();
            zip.by_name("word/document.xml").unwrap().read_to_string(&mut xml).unwrap();
            let margins = xml.split("<w:tblCellMar>").nth(1).unwrap().split("</w:tblCellMar>").next().unwrap();
            assert!(!margins.contains("<w:left") && !margins.contains("<w:top") && !margins.contains("<w:bottom"));
            assert_eq!(margins.contains(r#"<w:right w:w="0""#), !sides.is_empty());
            doc = wordcraft_docx::read(&bytes).unwrap();
        }
    }
}

#[test]
fn table_margin_complete_zero() {
    let sides = r#"<w:top w:w="0" w:type="dxa"/><w:start w:w="0" w:type="dxa"/><w:bottom w:w="0" w:type="dxa"/><w:end w:w="0" w:type="dxa"/>"#;
    let mut doc = wordcraft_docx::read(&fixture(sides)).unwrap();
    for _ in 0..2 {
        let props = &doc.body[0].as_table().unwrap().props;
        assert_eq!(props.cell_margins, Some([0.0; 4]));
        assert_eq!(props.partial_cell_margins, None);
        doc = wordcraft_docx::read(&wordcraft_docx::write(&doc).unwrap()).unwrap();
    }
}
