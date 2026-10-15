//! "Collapsed by default" headings (#1745): the [MS-DOCX] `w15:collapsed` paragraph property
//! (namespace `http://schemas.microsoft.com/office/word/2012/wordml`, an on/off child of
//! `w:pPr`). Read from any prefix bound to that namespace, written back as `<w15:collapsed/>`
//! with `w15` declared and listed in `mc:Ignorable`. Fixtures are hand-written.

use std::io::{Read, Write};

use wordcraft_doc::Document;

fn zip(entries: &[(&str, &[u8])]) -> Vec<u8> {
    let mut zw = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    for (name, bytes) in entries {
        zw.start_file(*name, zip::write::SimpleFileOptions::default()).unwrap();
        zw.write_all(bytes).unwrap();
    }
    zw.finish().unwrap().into_inner()
}

fn docx(body: &str) -> Vec<u8> {
    let root = r#"<?xml version="1.0" encoding="UTF-8"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="word/document.xml"/></Relationships>"#;
    let doc = format!(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main" xmlns:x15="http://schemas.microsoft.com/office/word/2012/wordml"><w:body>{body}</w:body></w:document>"#
    );
    zip(&[("_rels/.rels", root.as_bytes()), ("word/document.xml", doc.as_bytes())])
}

fn flags(d: &Document) -> Vec<Option<bool>> {
    d.body.iter().filter_map(|b| b.as_para()).map(|p| p.props.collapsed).collect()
}

#[test]
fn collapsed_reads_and_round_trips() {
    let body = r#"<w:p><w:pPr><w:outlineLvl w:val="0"/><x15:collapsed/></w:pPr><w:r><w:t>Intro</w:t></w:r></w:p>
        <w:p><w:r><w:t>Body</w:t></w:r></w:p>
        <w:p><w:pPr><w:outlineLvl w:val="1"/><x15:collapsed w:val="0"/></w:pPr><w:r><w:t>Off</w:t></w:r></w:p>"#;
    let d = wordcraft_docx::read(&docx(body)).unwrap();
    assert_eq!(flags(&d), vec![Some(true), None, Some(false)]);

    let mut z = zip::ZipArchive::new(std::io::Cursor::new(wordcraft_docx::write(&d).unwrap())).unwrap();
    let mut xml = String::new();
    z.by_name("word/document.xml").unwrap().read_to_string(&mut xml).unwrap();
    assert!(xml.contains("<w15:collapsed/>"), "{xml}");
    assert!(xml.contains(r#"xmlns:w15="http://schemas.microsoft.com/office/word/2012/wordml""#));
    let ignorable = xml.split("mc:Ignorable=\"").nth(1).and_then(|s| s.split('"').next()).unwrap_or("");
    assert!(ignorable.split_whitespace().any(|p| p == "w15"), "mc:Ignorable {ignorable:?}");
    let back = wordcraft_docx::read(&wordcraft_docx::write(&d).unwrap()).unwrap();
    assert_eq!(flags(&back), flags(&d));
}

/// Unset stays unset: nothing is written for ordinary paragraphs.
#[test]
fn no_flag_writes_nothing() {
    let d = Document::from_text("One\nTwo");
    let mut z = zip::ZipArchive::new(std::io::Cursor::new(wordcraft_docx::write(&d).unwrap())).unwrap();
    let mut xml = String::new();
    z.by_name("word/document.xml").unwrap().read_to_string(&mut xml).unwrap();
    assert!(!xml.contains("collapsed"));
}

/// A style can carry the flag (`w:style/w:pPr/w15:collapsed`): read, written back in styles.xml.
#[test]
fn a_style_carrying_the_flag_round_trips() {
    let mut d = Document::from_text("Intro\nBody");
    if let Some(st) = d.styles.styles.iter_mut().find(|s| s.id == "Heading1") {
        st.para.collapsed = Some(true);
    }
    let bytes = wordcraft_docx::write(&d).unwrap();
    let mut z = zip::ZipArchive::new(std::io::Cursor::new(bytes.clone())).unwrap();
    let mut xml = String::new();
    z.by_name("word/styles.xml").unwrap().read_to_string(&mut xml).unwrap();
    let h1 = xml.split("w:styleId=\"Heading1\"").nth(1).and_then(|s| s.split("</w:style>").next()).unwrap_or("");
    assert!(h1.contains("<w15:collapsed/>"), "{h1}");
    let back = wordcraft_docx::read(&bytes).unwrap();
    assert_eq!(back.styles.styles.iter().find(|s| s.id == "Heading1").and_then(|s| s.para.collapsed), Some(true));
}
