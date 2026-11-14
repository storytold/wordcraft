//! `settings.xml` (ODF 1.3 §3.10): LibreOffice's layout compatibility switches. A file's own are
//! kept through a save; a document from elsewhere gets those LibreOffice uses for Word files, as
//! WordCraft lays text out as Word does. Tabs are always written measured from the margin.

use crate::odt::tests::{NS_T, package};
use crate::odt::{export, import};
use wordcraft_doc::Document;

fn settings_of(bytes: &[u8]) -> String {
    let mut z = zip::ZipArchive::new(std::io::Cursor::new(bytes)).unwrap();
    let mut s = String::new();
    std::io::Read::read_to_string(&mut z.by_name("settings.xml").unwrap(), &mut s).unwrap();
    s
}

fn item(settings: &str, name: &str) -> Option<String> {
    let at = settings.find(&format!(r#"config:name="{name}""#))?;
    let rest = &settings[at..];
    let v = rest.split_once('>')?.1;
    Some(v.split('<').next()?.to_string())
}

#[test]
fn a_new_document_gets_word_compatible_layout() {
    let s = settings_of(&export(&Document::new()).unwrap());
    assert_eq!(item(&s, "TabsRelativeToIndent").as_deref(), Some("false"));
    assert_eq!(item(&s, "AddParaSpacingToTableCells").as_deref(), Some("true"));
    assert_eq!(item(&s, "TabOverMargin").as_deref(), Some("false"));
}

#[test]
fn a_files_own_settings_survive_a_save() {
    let settings = r#"<?xml version="1.0" encoding="UTF-8"?><office:document-settings xmlns:office="urn:oasis:names:tc:opendocument:xmlns:office:1.0" xmlns:config="urn:oasis:names:tc:opendocument:xmlns:config:1.0"><office:settings><config:config-item-set config:name="ooo:configuration-settings"><config:config-item config:name="AddParaTableSpacing" config:type="boolean">true</config:config-item><config:config-item config:name="TabsRelativeToIndent" config:type="boolean">true</config:config-item></config:config-item-set></office:settings></office:document-settings>"#;
    let content = format!(
        r#"<office:document-content {NS_T}><office:body><office:text><text:p>x</text:p></office:text></office:body></office:document-content>"#
    );
    let mut z = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    let o = zip::write::SimpleFileOptions::default();
    for (n, d) in [("mimetype", "application/vnd.oasis.opendocument.text"), ("content.xml", content.as_str()), ("settings.xml", settings)] {
        z.start_file(n, o).unwrap();
        std::io::Write::write_all(&mut z, d.as_bytes()).unwrap();
    }
    let d = import(&z.finish().unwrap().into_inner()).unwrap();
    let s = settings_of(&export(&d).unwrap());
    assert_eq!(item(&s, "AddParaTableSpacing").as_deref(), Some("true"), "{s}");
    // Not Word's switches over the file's own.
    assert_eq!(item(&s, "AddParaSpacingToTableCells"), None);
    // Tabs measured as the file measured them: from the indent here (its default stops too).
    assert_eq!(item(&s, "TabsRelativeToIndent").as_deref(), Some("true"));
    let _ = package;
}
