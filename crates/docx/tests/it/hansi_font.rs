//! `w:rFonts/@w:hAnsi` is the font of a run's non-ASCII Latin text and `w:ascii` the font of its
//! ASCII text (ECMA-376 Part 1 §17.3.2.26); each inherits on its own. Synthetic packages, written
//! here; no external documents or installed fonts are needed.

use std::io::Write;
use wordcraft_doc::{Document, Paragraph};

fn package(run_fonts: &str) -> Vec<u8> {
    let ns = r#"xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main""#;
    let root = r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="r1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="word/document.xml"/></Relationships>"#;
    let rels = r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="s1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/styles" Target="styles.xml"/></Relationships>"#;
    let document =
        format!(r#"<w:document {ns}><w:body><w:p><w:r><w:rPr>{run_fonts}</w:rPr><w:t>the “quoted” text</w:t></w:r></w:p></w:body></w:document>"#);
    let defaults = r#"<w:rFonts w:ascii="Source Serif 4" w:hAnsi="Source Serif 4"/>"#;
    let styles = format!("<w:styles {ns}><w:docDefaults><w:rPrDefault><w:rPr>{defaults}</w:rPr></w:rPrDefault></w:docDefaults></w:styles>");
    let mut z = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    for (name, data) in [
        ("_rels/.rels", root),
        ("word/_rels/document.xml.rels", rels),
        ("word/document.xml", document.as_str()),
        ("word/styles.xml", styles.as_str()),
    ] {
        z.start_file(name, zip::write::SimpleFileOptions::default()).unwrap();
        z.write_all(data.as_bytes()).unwrap();
    }
    z.finish().unwrap().into_inner()
}

fn paragraph(d: &Document) -> &Paragraph {
    d.body.iter().find_map(|b| b.as_para()).unwrap()
}

/// The resolved (ascii, hAnsi) fonts of the paragraph's first character.
fn fonts(d: &Document) -> (String, Option<String>) {
    let p = paragraph(d);
    let c = d.styles.resolve_char(p.props.style.as_deref(), p.props_of_char(0));
    (c.font, c.font_hansi)
}

fn document_xml(bytes: &[u8]) -> String {
    let mut z = zip::ZipArchive::new(std::io::Cursor::new(bytes)).unwrap();
    std::io::read_to_string(z.by_name("word/document.xml").unwrap()).unwrap()
}

#[test]
fn hansi_alone_keeps_the_inherited_ascii_font() {
    let d = wordcraft_docx::read(&package(r#"<w:rFonts w:hAnsi="JetBrains Mono"/>"#)).unwrap();
    let c = &paragraph(&d).runs[0].props;
    assert_eq!((c.font.as_deref(), c.font_hansi.as_deref()), (None, Some("JetBrains Mono")));
    assert_eq!(fonts(&d), ("Source Serif 4".into(), Some("JetBrains Mono".into())));
}

#[test]
fn distinct_ascii_and_hansi_fonts_are_both_read() {
    let d = wordcraft_docx::read(&package(r#"<w:rFonts w:ascii="Inter" w:hAnsi="JetBrains Mono"/>"#)).unwrap();
    assert_eq!(fonts(&d), ("Inter".into(), Some("JetBrains Mono".into())));
    // The same font for both is one font.
    let d = wordcraft_docx::read(&package(r#"<w:rFonts w:ascii="Inter" w:hAnsi="Inter"/>"#)).unwrap();
    assert_eq!(fonts(&d), ("Inter".into(), None));
    // A run naming only its ASCII font sets the whole run in it, as before.
    let d = wordcraft_docx::read(&package(r#"<w:rFonts w:ascii="Inter"/>"#)).unwrap();
    assert_eq!(fonts(&d), ("Inter".into(), None));
}

#[test]
fn ascii_and_hansi_fonts_round_trip() {
    for (run, ascii, hansi) in [
        (r#"<w:rFonts w:hAnsi="JetBrains Mono"/>"#, None, Some(r#"w:hAnsi="JetBrains Mono""#)),
        (r#"<w:rFonts w:ascii="Inter" w:hAnsi="JetBrains Mono"/>"#, Some(r#"w:ascii="Inter""#), Some(r#"w:hAnsi="JetBrains Mono""#)),
        (r#"<w:rFonts w:ascii="Inter" w:hAnsi="Inter"/>"#, Some(r#"w:ascii="Inter""#), Some(r#"w:hAnsi="Inter""#)),
    ] {
        let d = wordcraft_docx::read(&package(run)).unwrap();
        let bytes = wordcraft_docx::write(&d).unwrap();
        let xml = document_xml(&bytes);
        assert_eq!(xml.contains("w:ascii="), ascii.is_some(), "{run}: {xml}");
        for a in ascii.into_iter().chain(hansi) {
            assert!(xml.contains(a), "{run}: {a} missing from {xml}");
        }
        let back = wordcraft_docx::read(&bytes).unwrap();
        assert_eq!(paragraph(&back).runs, paragraph(&d).runs, "{run}");
        assert_eq!(fonts(&back), fonts(&d), "{run}");
    }
}
