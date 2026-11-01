//! The document default font size (`w:docDefaults/w:rPrDefault/w:rPr/w:sz`) across a save and
//! reopen. An unset default is resolved as `DEFAULT_FONT_SIZE`; the writer must say so, because
//! the reader reads an omitted size as 10 pt.
use std::io::{Read, Write};
use wordcraft_doc::Document;
use wordcraft_doc::props::{CharProps, DEFAULT_FONT_SIZE};
fn resolved(d: &Document) -> f32 {
    d.styles.resolve_char(None, &CharProps::default()).size
}
fn styles_xml(bytes: &[u8]) -> String {
    let mut archive = zip::ZipArchive::new(std::io::Cursor::new(bytes)).unwrap();
    let mut xml = String::new();
    archive.by_name("word/styles.xml").unwrap().read_to_string(&mut xml).unwrap();
    xml
}
fn defaults(xml: &str) -> &str {
    xml.split("<w:docDefaults>").nth(1).unwrap().split("</w:docDefaults>").next().unwrap()
}
#[test]
fn an_unset_default_size_keeps_its_effective_size_across_save_and_reopen() {
    let mut d = Document::new();
    d.styles.default_chr.size = None;
    assert_eq!(resolved(&d), DEFAULT_FONT_SIZE);
    let bytes = wordcraft_docx::write(&d).unwrap();
    assert!(defaults(&styles_xml(&bytes)).contains(r#"<w:sz w:val="22"/>"#));
    let reopened = wordcraft_docx::read(&bytes).unwrap();
    assert_eq!(resolved(&reopened), DEFAULT_FONT_SIZE);
    // Saving again changes nothing further.
    let again = wordcraft_docx::read(&wordcraft_docx::write(&reopened).unwrap()).unwrap();
    assert_eq!(resolved(&again), DEFAULT_FONT_SIZE);
}
#[test]
fn an_explicit_default_size_is_written_as_it_is() {
    for size in [8.0, 10.0, 11.0, 12.5, 18.0] {
        let mut d = Document::new();
        d.styles.default_chr.size = Some(size);
        let bytes = wordcraft_docx::write(&d).unwrap();
        assert_eq!(resolved(&wordcraft_docx::read(&bytes).unwrap()), size);
    }
}
#[test]
fn a_file_that_omits_the_default_size_is_still_read_as_ten_points() {
    // The reader is unchanged: only what WordCraft writes differs. A package without a size
    // anywhere keeps its established reading.
    let root = r#"<?xml version="1.0" encoding="UTF-8"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="word/document.xml"/></Relationships>"#;
    let doc = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:body><w:p><w:r><w:t>x</w:t></w:r></w:p></w:body></w:document>"#;
    let styles = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><w:styles xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:docDefaults><w:rPrDefault><w:rPr/></w:rPrDefault></w:docDefaults></w:styles>"#;
    let rels = r#"<?xml version="1.0" encoding="UTF-8"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/styles" Target="styles.xml"/></Relationships>"#;
    let mut zw = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    for (name, body) in [("_rels/.rels", root), ("word/document.xml", doc), ("word/_rels/document.xml.rels", rels), ("word/styles.xml", styles)] {
        zw.start_file(name, zip::write::SimpleFileOptions::default()).unwrap();
        zw.write_all(body.as_bytes()).unwrap();
    }
    let d = wordcraft_docx::read(&zw.finish().unwrap().into_inner()).unwrap();
    assert_eq!(d.styles.default_chr.size, Some(10.0));
}
#[test]
fn an_unset_default_size_does_not_take_a_styles_size() {
    // The written default is the resolver's own fallback, not what the Normal style resolves to,
    // so a style that sets a size keeps it and one that does not still falls back to the default.
    let mut d = Document::new();
    d.styles.default_chr.size = None;
    if let Some(normal) = d.styles.get_mut("Normal") {
        normal.chr.size = Some(14.0);
    }
    let reopened = wordcraft_docx::read(&wordcraft_docx::write(&d).unwrap()).unwrap();
    assert_eq!(reopened.styles.default_chr.size, Some(DEFAULT_FONT_SIZE));
    assert_eq!(resolved(&reopened), resolved(&d));
}
