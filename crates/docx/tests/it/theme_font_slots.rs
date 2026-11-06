//! Theme font slots of the East Asian and complex-script fonts (`w:rFonts/@w:eastAsiaTheme`,
//! `@w:cstheme`, ISO/IEC 29500-1 §17.3.2.26). Word picks those fonts through the slot, so a save
//! that drops it changes the font of East Asian punctuation and, with it, Word's line breaks.

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

fn rfonts(xml: &str) -> Vec<String> {
    xml.match_indices("<w:rFonts").map(|(i, _)| xml[i..].split('>').next().unwrap_or("").to_string()).collect()
}

#[test]
fn theme_font_slots_survive_a_save() {
    // As pandoc writes them: theme slots only, in the document defaults, and on a run.
    let styles = r#"<w:docDefaults><w:rPrDefault><w:rPr><w:rFonts w:asciiTheme="minorHAnsi" w:eastAsiaTheme="minorEastAsia" w:hAnsiTheme="minorHAnsi" w:cstheme="minorBidi"/><w:lang w:val="en-US" w:eastAsia="zh-CN"/></w:rPr></w:rPrDefault></w:docDefaults>"#;
    let body = r#"<w:p><w:r><w:rPr><w:rFonts w:eastAsiaTheme="majorEastAsia" w:cstheme="bogus"/></w:rPr><w:t>“Quoted” text</w:t></w:r></w:p>"#;
    let d = wordcraft_docx::read(&docx(body, styles)).unwrap();
    let run = &d.body[0].as_para().unwrap().runs[0];
    assert_eq!(run.props.font_ea_theme.as_deref(), Some("majorEastAsia"));
    assert_eq!(run.props.font_cs_theme, None, "a name outside ST_Theme is not kept");

    let out = wordcraft_docx::write(&d).unwrap();
    let styles_out = rfonts(&part(&out, "word/styles.xml"));
    assert!(
        styles_out.iter().any(|f| f.contains(r#"w:eastAsiaTheme="minorEastAsia""#) && f.contains(r#"w:cstheme="minorBidi""#)),
        "the document defaults keep their theme slots: {styles_out:?}"
    );
    let doc_out = rfonts(&part(&out, "word/document.xml"));
    assert!(doc_out.iter().any(|f| f.contains(r#"w:eastAsiaTheme="majorEastAsia""#)), "the run keeps its slot: {doc_out:?}");
}

#[test]
fn a_named_east_asian_font_replaces_the_theme_slot() {
    let mut base = wordcraft_doc::props::CharProps { font_ea_theme: Some("minorEastAsia".into()), ..Default::default() };
    base.overlay(&wordcraft_doc::props::CharProps { font_ea: Some("SimSun".into()), ..Default::default() });
    assert_eq!(base.font_ea.as_deref(), Some("SimSun"));
    assert_eq!(base.font_ea_theme, None, "the slot would otherwise win over the font just chosen");
}
