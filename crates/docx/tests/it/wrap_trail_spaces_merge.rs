//! `w:wrapTrailSpaces` on the settings merge path: when a file's settings part holds settings the
//! model doesn't own (here `w:rsids` and other `w:compat` switches), the writer merges the generated
//! part into the original one. The switch must still follow `Settings::wrap_trail_spaces` there:
//! turned off, it goes; turned on, it is inserted at its CT_Compat place (ECMA-376 Part 1, wml.xsd
//! CT_Compat: after `w:doNotExpandShiftReturn`, before `w:doNotUseHTMLParagraphAutoSpacing`,
//! `w:useFELayout` and the `w:compatSetting`s). Synthetic fixtures written for these tests.
use std::collections::BTreeMap;
use std::io::{Cursor, Read, Write};

const W_NS: &str = r#"xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships""#;
const RT: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships/";
const MODE: &str = r#"<w:compatSetting w:name="compatibilityMode" w:uri="http://schemas.microsoft.com/office/word" w:val="15"/>"#;
const RSIDS: &str = r#"<w:rsids><w:rsidRoot w:val="00112233"/></w:rsids>"#;

fn zip(entries: &[(&str, String)]) -> Vec<u8> {
    let mut z = zip::ZipWriter::new(Cursor::new(Vec::new()));
    for (name, text) in entries {
        z.start_file(*name, zip::write::SimpleFileOptions::default()).unwrap();
        z.write_all(text.as_bytes()).unwrap();
    }
    z.finish().unwrap().into_inner()
}

fn unzip(bytes: &[u8]) -> BTreeMap<String, String> {
    let mut z = zip::ZipArchive::new(Cursor::new(bytes)).unwrap();
    let mut parts = BTreeMap::new();
    for i in 0..z.len() {
        let mut f = z.by_index(i).unwrap();
        let mut s = String::new();
        if f.read_to_string(&mut s).is_ok() {
            parts.insert(f.name().to_string(), s);
        }
    }
    parts
}

/// A package whose settings part holds `compat` (the children of `w:compat`) and an rsid table.
fn fixture(compat: &str) -> Vec<u8> {
    let root_rels = format!(
        r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="{RT}officeDocument" Target="word/document.xml"/></Relationships>"#
    );
    let doc_rels = format!(
        r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="{RT}settings" Target="settings.xml"/></Relationships>"#
    );
    zip(&[
        ("_rels/.rels", root_rels),
        ("word/document.xml", format!(r#"<w:document {W_NS}><w:body><w:p><w:r><w:t>Body</w:t></w:r></w:p></w:body></w:document>"#)),
        ("word/_rels/document.xml.rels", doc_rels),
        ("word/settings.xml", format!(r#"<w:settings {W_NS}><w:defaultTabStop w:val="720"/><w:compat>{compat}</w:compat>{RSIDS}</w:settings>"#)),
    ])
}

/// Open `compat`, set the switch to `on`, save; the saved settings part and the reopened flag.
fn flip(compat: &str, on: bool) -> (String, bool) {
    let mut d = wordcraft_docx::read(&fixture(compat)).unwrap();
    d.settings.wrap_trail_spaces = on;
    let bytes = wordcraft_docx::write(&d).unwrap();
    let xml = unzip(&bytes).remove("word/settings.xml").unwrap_or_default();
    let again = wordcraft_docx::read(&bytes).unwrap();
    (xml, again.settings.wrap_trail_spaces)
}

fn at(xml: &str, e: &str) -> usize {
    xml.find(e).unwrap_or_else(|| panic!("{e} missing in {xml}"))
}

#[test]
fn turning_it_off_removes_the_original_switch() {
    let compat = format!("<w:wrapTrailSpaces/><w:useFELayout/>{MODE}");
    assert!(wordcraft_docx::read(&fixture(&compat)).unwrap().settings.wrap_trail_spaces);
    let (xml, reopened) = flip(&compat, false);
    assert!(!xml.contains("wrapTrailSpaces"), "{xml}");
    assert!(xml.contains("<w:useFELayout/>") && xml.contains(r#"<w:rsidRoot w:val="00112233"/>"#), "{xml}");
    assert!(!reopened);
}

#[test]
fn turning_it_on_inserts_it_in_ct_compat_order() {
    let (xml, reopened) = flip(&format!("<w:useFELayout/>{MODE}"), true);
    assert_eq!(xml.matches("<w:wrapTrailSpaces").count(), 1, "{xml}");
    assert!(at(&xml, "<w:wrapTrailSpaces") < at(&xml, "<w:useFELayout/>"), "{xml}");
    assert!(at(&xml, "<w:wrapTrailSpaces") < at(&xml, "<w:compatSetting"), "{xml}");
    assert!(xml.contains(r#"<w:rsidRoot w:val="00112233"/>"#), "{xml}");
    assert!(reopened);
}

#[test]
fn inserted_between_the_switches_around_it() {
    let (xml, reopened) = flip(&format!("<w:doNotExpandShiftReturn/><w:doNotUseHTMLParagraphAutoSpacing/>{MODE}"), true);
    assert_eq!(xml.matches("<w:wrapTrailSpaces").count(), 1, "{xml}");
    assert!(at(&xml, "<w:doNotExpandShiftReturn") < at(&xml, "<w:wrapTrailSpaces"), "{xml}");
    assert!(at(&xml, "<w:wrapTrailSpaces") < at(&xml, "<w:doNotUseHTMLParagraphAutoSpacing"), "{xml}");
    assert!(reopened);
}

#[test]
fn an_explicit_off_is_replaced_when_turned_on() {
    let (xml, reopened) = flip(r#"<w:wrapTrailSpaces w:val="0"/><w:useFELayout/>"#, true);
    assert_eq!(xml.matches("<w:wrapTrailSpaces").count(), 1, "{xml}");
    assert!(!xml.contains(r#"<w:wrapTrailSpaces w:val="0"/>"#), "{xml}");
    assert!(reopened);
}

#[test]
fn an_unchanged_switch_is_written_once() {
    let (xml, reopened) = flip(&format!("<w:wrapTrailSpaces/><w:useFELayout/>{MODE}"), true);
    assert_eq!(xml.matches("<w:wrapTrailSpaces").count(), 1, "{xml}");
    assert!(reopened);
}
