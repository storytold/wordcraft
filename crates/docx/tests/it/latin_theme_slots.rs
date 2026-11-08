//! Theme font slots of Latin text (`w:rFonts/@w:asciiTheme`, `@w:hAnsiTheme`, ISO/IEC 29500-1
//! §17.3.2.26). Saved as the font they name, the text looked the same in Word but no longer
//! followed a change of the theme's fonts. Saving writes a slot back only while it still names the
//! text's font in the document's theme, so a font chosen later is never hidden by it.

use std::io::{Read, Write};

const THEME: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><a:theme xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" name="T"><a:themeElements><a:fontScheme name="T"><a:majorFont><a:latin typeface="Georgia"/><a:ea typeface=""/><a:cs typeface=""/></a:majorFont><a:minorFont><a:latin typeface="Verdana"/><a:ea typeface=""/><a:cs typeface=""/></a:minorFont></a:fontScheme></a:themeElements></a:theme>"#;

fn docx(body: &str, styles: &str) -> Vec<u8> {
    let root = r#"<?xml version="1.0" encoding="UTF-8"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="word/document.xml"/></Relationships>"#;
    let doc_rels = r#"<?xml version="1.0" encoding="UTF-8"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/styles" Target="styles.xml"/><Relationship Id="rId2" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/theme" Target="theme/theme1.xml"/></Relationships>"#;
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
        ("word/theme/theme1.xml", THEME.to_string()),
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

// As pandoc and Word write them: the minor font in the defaults, the major one on a run.
const STYLES: &str = r#"<w:docDefaults><w:rPrDefault><w:rPr><w:rFonts w:asciiTheme="minorHAnsi" w:hAnsiTheme="minorHAnsi" w:eastAsiaTheme="minorEastAsia" w:cstheme="minorBidi"/></w:rPr></w:rPrDefault></w:docDefaults>"#;
const BODY: &str = r#"<w:p><w:r><w:rPr><w:rFonts w:asciiTheme="majorHAnsi" w:hAnsiTheme="majorHAnsi"/></w:rPr><w:t>Heading text</w:t></w:r></w:p>"#;

#[test]
fn latin_theme_slots_survive_a_save() {
    let d = wordcraft_docx::read(&docx(BODY, STYLES)).unwrap();
    assert_eq!(d.styles.default_chr.font.as_deref(), Some("Verdana"), "layout still reads the font the slot names");
    assert_eq!(d.styles.default_chr.font_ascii_theme, Some(wordcraft_doc::props::ThemeFont::MinorHAnsi));
    let run = &d.body[0].as_para().unwrap().runs[0];
    assert_eq!((run.props.font.as_deref(), run.props.font_hansi_theme), (Some("Georgia"), Some(wordcraft_doc::props::ThemeFont::MajorHAnsi)));

    let out = wordcraft_docx::write(&d).unwrap();
    let styles = rfonts(&part(&out, "word/styles.xml"));
    assert!(styles.iter().any(|f| f.contains(r#"w:asciiTheme="minorHAnsi""#) && f.contains(r#"w:hAnsiTheme="minorHAnsi""#)), "{styles:?}");
    let doc = rfonts(&part(&out, "word/document.xml"));
    assert!(doc.iter().any(|f| f.contains(r#"w:asciiTheme="majorHAnsi""#) && f.contains(r#"w:hAnsiTheme="majorHAnsi""#)), "{doc:?}");
    // And again after a second round trip.
    let again = wordcraft_docx::read(&out).unwrap();
    assert_eq!(again.styles.default_chr.font_ascii_theme, Some(wordcraft_doc::props::ThemeFont::MinorHAnsi));
}

#[test]
fn a_font_chosen_later_is_not_hidden_by_the_slot() {
    let mut d = wordcraft_docx::read(&docx(BODY, STYLES)).unwrap();
    // Set directly, as the font commands do, without touching the slots.
    if let Some(wordcraft_doc::Block::Para(p)) = d.body.get_mut(0).map(std::sync::Arc::make_mut) {
        p.runs[0].props.font = Some("Courier New".into());
    }
    let doc = rfonts(&part(&wordcraft_docx::write(&d).unwrap(), "word/document.xml"));
    assert!(doc.iter().any(|f| f.contains(r#"w:ascii="Courier New""#)), "{doc:?}");
    assert!(!doc.iter().any(|f| f.contains("w:asciiTheme") || f.contains("w:hAnsiTheme")), "the slot would show Georgia in Word: {doc:?}");
}

#[test]
fn a_theme_font_change_without_the_text_drops_the_slot() {
    let mut d = wordcraft_docx::read(&docx(BODY, STYLES)).unwrap();
    // The theme's minor font changes but the defaults keep Verdana: the slot no longer names it.
    d.settings.minor_font = "Arial".into();
    let styles = rfonts(&part(&wordcraft_docx::write(&d).unwrap(), "word/styles.xml"));
    assert!(!styles.iter().any(|f| f.contains(r#"w:asciiTheme="minorHAnsi""#)), "{styles:?}");
    assert!(styles.iter().any(|f| f.contains(r#"w:ascii="Verdana""#)), "{styles:?}");
}

#[test]
fn a_named_font_overlaid_replaces_the_latin_slots() {
    use wordcraft_doc::props::CharProps;
    let mut base = CharProps {
        font: Some("Verdana".into()),
        font_ascii_theme: Some(wordcraft_doc::props::ThemeFont::MinorHAnsi),
        font_hansi_theme: Some(wordcraft_doc::props::ThemeFont::MinorHAnsi),
        ..Default::default()
    };
    base.overlay(&CharProps { font: Some("Courier New".into()), ..Default::default() });
    assert_eq!((base.font_ascii_theme, base.font_hansi_theme), (None, None));
    let mut themed = CharProps { font: Some("Verdana".into()), ..Default::default() };
    themed.overlay(&CharProps {
        font: Some("Georgia".into()),
        font_ascii_theme: Some(wordcraft_doc::props::ThemeFont::MajorHAnsi),
        ..Default::default()
    });
    assert_eq!(themed.font_ascii_theme, Some(wordcraft_doc::props::ThemeFont::MajorHAnsi));
}
