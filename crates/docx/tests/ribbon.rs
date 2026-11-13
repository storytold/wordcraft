//! The document's ribbon part (customUI): modelled when it is tabs, groups and macro buttons,
//! kept unchanged when it is richer, written as `customUI/customUI14.xml`. Hand-written fixtures
//! from the published custom UI schema, no files produced by Word.

use std::collections::BTreeMap;
use std::io::{Read, Write};

use wordcraft_doc::{Document, Ribbon, RibbonButton, RibbonGroup, RibbonTab};
use wordcraft_docx::custom_ui::{has_kept_ribbon, kept_part, parse_buttons, parse_exact};

const W: &str = "http://schemas.openxmlformats.org/wordprocessingml/2006/main";
const UI14_REL: &str = "http://schemas.microsoft.com/office/2007/relationships/ui/extensibility";
const UI12_REL: &str = "http://schemas.microsoft.com/office/2006/relationships/ui/extensibility";

fn zip(entries: &[(&str, Vec<u8>)]) -> Vec<u8> {
    let mut zw = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    for (name, bytes) in entries {
        zw.start_file(*name, zip::write::SimpleFileOptions::default()).unwrap();
        zw.write_all(bytes).unwrap();
    }
    zw.finish().unwrap().into_inner()
}

fn unzip(bytes: &[u8]) -> BTreeMap<String, Vec<u8>> {
    let mut z = zip::ZipArchive::new(std::io::Cursor::new(bytes)).unwrap();
    let mut out = BTreeMap::new();
    for i in 0..z.len() {
        let mut f = z.by_index(i).unwrap();
        let mut b = Vec::new();
        f.read_to_end(&mut b).unwrap();
        assert!(out.insert(f.name().to_string(), b).is_none(), "duplicate part {}", f.name());
    }
    out
}

fn text(files: &BTreeMap<String, Vec<u8>>, name: &str) -> String {
    String::from_utf8(files.get(name).unwrap_or_else(|| panic!("no {name} in {:?}", files.keys())).clone()).unwrap()
}

fn rels(list: &[(&str, &str, &str)]) -> Vec<u8> {
    let mut s =
        String::from(r#"<?xml version="1.0" encoding="UTF-8"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">"#);
    for (id, ty, target) in list {
        s.push_str(&format!(r#"<Relationship Id="{id}" Type="{ty}" Target="{target}"/>"#));
    }
    s.push_str("</Relationships>");
    s.into_bytes()
}

/// A one-paragraph package whose root relates `ribbons` (relationship type, part name, content),
/// plus `extra` parts.
fn package(ribbons: &[(&str, &str, &str)], extra: &[(&str, &[u8])]) -> Vec<u8> {
    let doc = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?><w:document xmlns:w="{W}"><w:body><w:p><w:r><w:t>Hi</w:t></w:r></w:p></w:body></w:document>"#
    );
    let ids: Vec<String> = (0..ribbons.len()).map(|i| format!("rIdUi{i}")).collect();
    let mut root = vec![("rIdMain", "http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument", "word/document.xml")];
    for (i, (kind, part, _)) in ribbons.iter().enumerate() {
        root.push((ids[i].as_str(), kind, part));
    }
    let mut entries: Vec<(&str, Vec<u8>)> = vec![("_rels/.rels", rels(&root)), ("word/document.xml", doc.into_bytes())];
    for (_, part, xml) in ribbons {
        entries.push((part, xml.as_bytes().to_vec()));
    }
    for (p, b) in extra {
        entries.push((p, b.to_vec()));
    }
    zip(&entries)
}

const SIMPLE: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<customUI xmlns="http://schemas.microsoft.com/office/2009/07/customui">
  <ribbon startFromScratch="false">
    <tabs>
      <tab id="tabTools" label="Tools">
        <group id="grpLetters" label="Letters">
          <button id="btnFill" label="Fill &amp; sign" size="large" imageMso="HappyFace" onAction="Module1.FillLetter" screentip="Fills the letter"/>
          <button id="btnClear" label="Clear" onAction="Clear" tag="all"/>
        </group>
      </tab>
      <tab idMso="TabReferences">
        <group id="grpQuick" label="Quick">
          <button id="btnHello" label="Hello" onAction="Hello"/>
        </group>
      </tab>
    </tabs>
  </ribbon>
</customUI>"#;

#[test]
fn a_ribbon_of_macro_buttons_is_modelled_and_round_trips() {
    let doc = wordcraft_docx::read(&package(&[(UI14_REL, "customUI/customUI14.xml", SIMPLE)], &[])).unwrap();
    let r = doc.ribbon.as_ref().expect("modelled");
    assert_eq!(r.tabs.len(), 2);
    assert_eq!(r.tabs[0].label, "Tools");
    assert_eq!(r.tabs[1].builtin.as_deref(), Some("TabReferences"));
    let fill = r.button("btnFill").unwrap();
    assert_eq!((fill.label.as_str(), fill.on_action.as_str(), fill.large), ("Fill & sign", "Module1.FillLetter", true));
    assert_eq!(fill.image.as_deref(), Some("HappyFace"));
    assert_eq!(fill.screentip.as_deref(), Some("Fills the letter"));
    assert_eq!(r.button("btnClear").unwrap().tag.as_deref(), Some("all"));
    // Modelled means not kept as well: one ribbon part on save, the model's.
    assert!(!has_kept_ribbon(&doc));
    assert!(doc.passthrough.keys().all(|k| !k.to_ascii_lowercase().contains("customui")), "{:?}", doc.passthrough.keys());

    let files = unzip(&wordcraft_docx::write(&doc).unwrap());
    let xml = text(&files, "customUI/customUI14.xml");
    assert!(xml.contains("2009/07/customui") && xml.contains("onAction=\"Module1.FillLetter\""), "{xml}");
    let root = text(&files, "_rels/.rels");
    assert!(root.contains(UI14_REL) && root.contains("customUI/customUI14.xml"), "{root}");
    assert_eq!(root.matches("ui/extensibility").count(), 1);
    assert!(text(&files, "[Content_Types].xml").contains("/customUI/customUI14.xml"));
    let back = wordcraft_docx::read(&zip_back(&files)).unwrap();
    assert_eq!(back.ribbon, doc.ribbon, "the ribbon survives a round trip");
}

fn zip_back(files: &BTreeMap<String, Vec<u8>>) -> Vec<u8> {
    let entries: Vec<(&str, Vec<u8>)> = files.iter().map(|(k, v)| (k.as_str(), v.clone())).collect();
    zip(&entries)
}

#[test]
fn the_office_2010_part_wins_over_the_office_2007_one() {
    let old = SIMPLE.replace("2009/07/customui", "2006/01/customui").replace("Fill &amp; sign", "Old label");
    let doc =
        wordcraft_docx::read(&package(&[(UI12_REL, "customUI/customUI.xml", &old), (UI14_REL, "customUI/customUI14.xml", SIMPLE)], &[])).unwrap();
    assert_eq!(doc.ribbon.as_ref().unwrap().button("btnFill").unwrap().label, "Fill & sign");
    let files = unzip(&wordcraft_docx::write(&doc).unwrap());
    assert!(!files.contains_key("customUI/customUI.xml"), "one ribbon part written");
    assert_eq!(text(&files, "_rels/.rels").matches("ui/extensibility").count(), 1);
    // The Office 2007 namespace alone is modelled too.
    let doc = wordcraft_docx::read(&package(&[(UI12_REL, "customUI/customUI.xml", &old)], &[])).unwrap();
    assert_eq!(doc.ribbon.as_ref().unwrap().button("btnFill").unwrap().label, "Old label");
}

#[test]
fn a_richer_ribbon_is_kept_unchanged_and_its_buttons_still_found() {
    // A menu, a load callback and a picture of its own: not modelled.
    let rich = r#"<customUI xmlns="http://schemas.microsoft.com/office/2009/07/customui" onLoad="RibbonLoaded"><ribbon><tabs><tab id="t" label="Mine"><group id="g" label="G"><menu id="m" label="More"><button id="b1" label="Inside" onAction="Inside"/></menu><button id="b2" image="logo" onAction="Logo"/><button idMso="Copy"/><button id="b3" label="Hidden" visible="false" onAction="Hidden"/></group></tab></tabs></ribbon></customUI>"#;
    let ui_rels = rels(&[("logo", "http://schemas.openxmlformats.org/officeDocument/2006/relationships/image", "images/logo.png")]);
    assert!(parse_exact(rich.as_bytes()).is_none());
    let doc = wordcraft_docx::read(&package(
        &[(UI14_REL, "customUI/customUI14.xml", rich)],
        &[("customUI/_rels/customUI14.xml.rels", &ui_rels), ("customUI/images/logo.png", b"\x89PNG not really")],
    ))
    .unwrap();
    assert!(doc.ribbon.is_none());
    assert!(has_kept_ribbon(&doc));
    assert_eq!(kept_part(&doc), Some(rich.as_bytes()));
    let found = parse_buttons(kept_part(&doc).unwrap());
    let labels: Vec<&str> = found.tabs[0].groups[0].buttons.iter().map(|b| b.label.as_str()).collect();
    assert_eq!(labels, ["Inside", "b2"], "menu buttons found; built-in commands and hidden buttons left out");

    // Kept byte for byte (under whatever part name the writer gave it), with its picture.
    let files = unzip(&wordcraft_docx::write(&doc).unwrap());
    let ui = files.keys().find(|n| n.to_ascii_lowercase().starts_with("customui/") && n.ends_with(".xml")).expect("kept");
    assert_eq!(text(&files, ui), rich);
    assert!(files.keys().any(|n| n.ends_with("logo.png")));
    // A modelled ribbon set meanwhile doesn't clash with the kept one.
    let mut doc = doc;
    doc.ribbon = Some(found);
    let files = unzip(&wordcraft_docx::write(&doc).unwrap());
    assert_eq!(text(&files, "_rels/.rels").matches("ui/extensibility").count(), 1);
}

#[test]
fn a_ribbon_is_written_for_a_new_document_and_not_when_empty() {
    let mut doc = Document::new();
    doc.ribbon = Some(Ribbon::default());
    assert!(!unzip(&wordcraft_docx::write(&doc).unwrap()).contains_key("customUI/customUI14.xml"), "an empty ribbon writes nothing");
    doc.ribbon = Some(Ribbon {
        tabs: vec![RibbonTab {
            builtin: Some("TabDeveloper".into()),
            groups: vec![RibbonGroup {
                id: "g1".into(),
                label: "My <macros>".into(),
                buttons: vec![RibbonButton { id: "b1".into(), label: "Run \"it\"".into(), on_action: "Module1.Go".into(), ..Default::default() }],
            }],
            ..Default::default()
        }],
    });
    let bytes = wordcraft_docx::write(&doc).unwrap();
    assert_eq!(wordcraft_docx::read(&bytes).unwrap().ribbon, doc.ribbon);
    // A macro-enabled package carries it too.
    let docm = wordcraft_docx::write_as(&doc, wordcraft_docx::Flavor::MacroDocument).unwrap();
    assert_eq!(wordcraft_docx::read(&docm).unwrap().ribbon, doc.ribbon);
}

#[test]
fn hostile_ribbon_parts_never_panic() {
    assert!(parse_exact(b"<customUI><ribbon startFromScratch=\"true\"/></customUI>").is_none());
    assert!(parse_exact(b"<notUI/>").is_none());
    assert!(parse_exact(b"<customUI><ribbon><tabs><tab id=\"a\" idMso=\"TabHome\"/></tabs></ribbon></customUI>").is_none(), "both ids");
    assert!(
        parse_exact(b"<customUI><ribbon><tabs><tab id=\"a\"><group id=\"g\"><button id=\"b\"/></group></tab></tabs></ribbon></customUI>").is_none(),
        "no onAction"
    );
    assert!(parse_buttons(b"<<<").tabs.is_empty(), "junk finds nothing");
    let deep = format!(
        "<customUI><ribbon><tabs><tab id=\"t\"><group id=\"g\">{}<button id=\"b\" onAction=\"X\"/>{}</group></tab></tabs></ribbon></customUI>",
        "<menu>".repeat(200),
        "</menu>".repeat(200)
    );
    assert!(parse_buttons(deep.as_bytes()).tabs.is_empty(), "buttons nested too deep are not looked for");
    let many = format!(
        "<customUI><ribbon><tabs><tab id=\"t\"><group id=\"g\">{}</group></tab></tabs></ribbon></customUI>",
        (0..1000).map(|i| format!("<button id=\"b{i}\" onAction=\"X\"/>")).collect::<String>()
    );
    assert!(parse_exact(many.as_bytes()).is_none(), "too many buttons to model");
    assert_eq!(parse_buttons(many.as_bytes()).tabs[0].groups[0].buttons.len(), wordcraft_doc::ribbon::MAX_RIBBON_ITEMS);
    // A ribbon relationship to a missing part, or to junk, reads as no ribbon.
    let doc = wordcraft_docx::read(&package(&[(UI14_REL, "customUI/customUI14.xml", "<<<not xml")], &[])).unwrap();
    assert!(doc.ribbon.is_none());
}
