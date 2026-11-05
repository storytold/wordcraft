//! Original synthetic custom XML stores, written from the public OOXML package contract.
use std::collections::BTreeMap;
use std::io::{Cursor, Read, Write};
use wordcraft_doc::{Path, StoryRef};

const REL_NS: &str = "http://schemas.openxmlformats.org/package/2006/relationships";
const RT: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships/";
const DATA: &str =
    "<?xml version=\"1.0\"?><c:customer xmlns:c=\"urn:original:customer\"><c:name>Atlas Studios</c:name><c:ref>CASE-17</c:ref></c:customer>";
const PROPS: &str = r#"<ds:datastoreItem xmlns:ds="http://schemas.openxmlformats.org/officeDocument/2006/customXml" ds:itemID="{22222222-2222-2222-2222-222222222222}"><ds:schemaRefs><ds:schemaRef ds:uri="urn:original:customer"/></ds:schemaRefs></ds:datastoreItem>"#;

fn rels(rows: &[(&str, &str, &str, bool)]) -> String {
    let mut out = format!(r#"<Relationships xmlns="{REL_NS}">"#);
    for (id, kind, target, external) in rows {
        let mode = if *external { r#" TargetMode="External""# } else { "" };
        out.push_str(&format!(r#"<Relationship Id="{id}" Type="{RT}{kind}" Target="{target}"{mode}/>"#));
    }
    out + "</Relationships>"
}

fn fixture() -> BTreeMap<String, Vec<u8>> {
    let doc = r#"<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:body><w:p><w:r><w:t>Independent edit.</w:t></w:r></w:p><w:p><w:sdt><w:sdtPr><w:id w:val="42"/><w:dataBinding w:xpath="/c:customer/c:name" w:prefixMappings="xmlns:c='urn:original:customer'" w:storeItemID="{22222222-2222-2222-2222-222222222222}"/><w:text/></w:sdtPr><w:sdtContent><w:r><w:t>Atlas Studios</w:t></w:r></w:sdtContent></w:sdt></w:p></w:body></w:document>"#;
    let types = r#"<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="xml" ContentType="application/xml"/><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Override PartName="/word/document.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml"/><Override PartName="/customXml/itemProps1.xml" ContentType="application/vnd.openxmlformats-officedocument.customXmlProperties+xml"/></Types>"#;
    [
        ("[Content_Types].xml", types.to_string()),
        ("_rels/.rels", rels(&[("office", "officeDocument", "word/document.xml", false)])),
        ("word/document.xml", doc.to_string()),
        ("word/_rels/document.xml.rels", rels(&[("data", "customXml", "../customXml/item1.xml", false)])),
        ("customXml/item1.xml", DATA.to_string()),
        ("customXml/itemProps1.xml", PROPS.to_string()),
        ("customXml/_rels/item1.xml.rels", rels(&[("properties", "customXmlProps", "itemProps1.xml", false)])),
    ]
    .into_iter()
    .map(|(p, s)| (p.into(), s.into_bytes()))
    .collect()
}

fn zip(parts: &BTreeMap<String, Vec<u8>>) -> Vec<u8> {
    let mut z = zip::ZipWriter::new(Cursor::new(Vec::new()));
    for (name, bytes) in parts {
        z.start_file(name, zip::write::SimpleFileOptions::default()).unwrap();
        z.write_all(bytes).unwrap();
    }
    z.finish().unwrap().into_inner()
}

fn unzip(bytes: &[u8]) -> BTreeMap<String, Vec<u8>> {
    let mut z = zip::ZipArchive::new(Cursor::new(bytes)).unwrap();
    let mut parts = BTreeMap::new();
    for i in 0..z.len() {
        let mut f = z.by_index(i).unwrap();
        let name = f.name().to_string();
        let mut bytes = Vec::new();
        f.read_to_end(&mut bytes).unwrap();
        assert!(parts.insert(name, bytes).is_none(), "duplicate ZIP entry");
    }
    parts
}

fn text(parts: &BTreeMap<String, Vec<u8>>, name: &str) -> String {
    String::from_utf8(parts[name].clone()).unwrap()
}

fn edited_cycles(input: &BTreeMap<String, Vec<u8>>, check: impl Fn(&BTreeMap<String, Vec<u8>>)) {
    let mut doc = wordcraft_docx::read(&zip(input)).unwrap();
    for _ in 0..2 {
        doc.para_mut(StoryRef::Body, &Path::top(0)).unwrap().insert_text(0, "Unrelated edit. ", &Default::default()).unwrap();
        let bytes = wordcraft_docx::write(&doc).unwrap();
        check(&unzip(&bytes));
        doc = wordcraft_docx::read(&bytes).unwrap();
    }
}

#[test]
fn bound_store_bytes_properties_and_relationships_survive_two_edited_saves() {
    edited_cycles(&fixture(), |p| {
        let document = text(p, "word/document.xml");
        assert!(document.contains("<w:dataBinding"), "the control keeps its store binding");
        assert!(document.contains(r#"w:storeItemID="{22222222-2222-2222-2222-222222222222}""#));
        assert!(document.contains(r#"w:xpath="/c:customer/c:name""#));
        assert_eq!(p.get("customXml/item1.xml").map(Vec::as_slice), Some(DATA.as_bytes()));
        assert_eq!(p.get("customXml/itemProps1.xml").map(Vec::as_slice), Some(PROPS.as_bytes()));
        assert!(text(p, "word/_rels/document.xml.rels").contains(r#"Target="../customXml/item1.xml""#));
        assert!(text(p, "customXml/_rels/item1.xml.rels").contains(r#"Id="properties""#));
        assert!(text(p, "customXml/_rels/item1.xml.rels").contains(r#"Target="itemProps1.xml""#));
        assert!(text(p, "[Content_Types].xml").contains("application/vnd.openxmlformats-officedocument.customXmlProperties+xml"));
    });
}

#[test]
fn package_and_document_roots_share_parts_and_keep_multiple_stores() {
    let mut p = fixture();
    p.insert(
        "_rels/.rels".into(),
        rels(&[("office", "officeDocument", "word/document.xml", false), ("data", "customXml", "customXml/item1.xml", false)]).into_bytes(),
    );
    p.insert(
        "word/_rels/document.xml.rels".into(),
        rels(&[("one", "customXml", "../customXml/item1.xml", false), ("two", "customXml", "../customXml/other.xml", false)]).into_bytes(),
    );
    p.insert("customXml/other.xml".into(), b"<other>Independent store</other>".to_vec());
    p.insert("customXml/_rels/other.xml.rels".into(), rels(&[("shared", "customXmlProps", "itemProps1.xml", false)]).into_bytes());
    // A cycle must not cause repeated parts, and an external schema stays external.
    p.insert(
        "customXml/_rels/itemProps1.xml.rels".into(),
        rels(&[("cycle", "customXml", "item1.xml", false), ("schema", "schema", "https://example.invalid/schema", true)]).into_bytes(),
    );
    edited_cycles(&p, |out| {
        assert_eq!(out.get("customXml/item1.xml").map(Vec::as_slice), Some(DATA.as_bytes()));
        assert_eq!(out.get("customXml/other.xml"), p.get("customXml/other.xml"));
        assert!(text(out, "_rels/.rels").contains(r#"Target="customXml/item1.xml""#));
        assert!(text(out, "word/_rels/document.xml.rels").contains(r#"Target="../customXml/other.xml""#));
        assert!(text(out, "customXml/_rels/itemProps1.xml.rels").contains(r#"TargetMode="External""#));
        assert!(text(out, "customXml/_rels/other.xml.rels").contains(r#"Target="itemProps1.xml""#));
    });
}

#[test]
fn bibliography_part_with_no_usable_sources_is_replaced_when_sources_are_added() {
    let mut p = fixture();
    // A `b:Sources` part whose only entry has no usable field: nothing is read from it.
    let garbage = r#"<b:Sources xmlns:b="http://schemas.openxmlformats.org/officeDocument/2006/bibliography"><b:Source><b:Author><b:NameList/></b:Author></b:Source></b:Sources>"#;
    p.insert("customXml/item2.xml".into(), garbage.as_bytes().to_vec());
    p.insert(
        "word/_rels/document.xml.rels".into(),
        rels(&[("data", "customXml", "../customXml/item1.xml", false), ("bib", "customXml", "../customXml/item2.xml", false)]).into_bytes(),
    );
    let mut doc = wordcraft_docx::read(&zip(&p)).unwrap();
    assert!(doc.sources.is_empty(), "{:?}", doc.sources);
    // Without sources of our own, the part rides along unchanged, like any other store.
    let out = unzip(&wordcraft_docx::write(&doc).unwrap());
    assert_eq!(out.get("customXml/item2.xml").map(Vec::as_slice), Some(garbage.as_bytes()));
    // Once the document has sources, the bibliography is written (to its `customXml/item1.xml`,
    // #532) and the store that held nothing usable is left out: no stale copy.
    doc.sources.push(wordcraft_doc::Source { tag: "Added".into(), title: "Added later".into(), ..Default::default() });
    let out = unzip(&wordcraft_docx::write(&doc).unwrap());
    let bib = text(&out, "customXml/item1.xml");
    assert!(bib.contains("<b:Tag>Added</b:Tag>"), "{bib}");
    assert!(!bib.contains("NameList"), "the old empty store's markup is gone: {bib}");
    assert!(out.contains_key("customXml/itemProps1.xml"));
    assert!(!out.contains_key("customXml/item2.xml"), "the stale store the file carried is not written beside the new one");
    assert!(!out.contains_key("customXml/item3.xml"));
    assert!(text(&out, "_rels/.rels").contains(r#"Target="customXml/item1.xml""#), "the bibliography is related from the package root");
}

#[test]
fn absent_and_external_stores_are_not_fetched_or_written() {
    let mut p = fixture();
    p.insert(
        "word/_rels/document.xml.rels".into(),
        rels(&[
            ("missing", "customXml", "../customXml/missing.xml", false),
            ("external", "customXml", "https://example.invalid/store.xml", true),
            ("valid", "customXml", "../customXml/item1.xml", false),
        ])
        .into_bytes(),
    );
    p.insert("customXml/_rels/item1.xml.rels".into(), rels(&[("missing", "customXmlProps", "missing.xml", false)]).into_bytes());
    edited_cycles(&p, |out| {
        assert_eq!(out.get("customXml/item1.xml").map(Vec::as_slice), Some(DATA.as_bytes()));
        let roots = text(out, "word/_rels/document.xml.rels");
        assert!(!roots.contains("missing.xml"));
        assert!(!roots.contains("example.invalid"));
        assert!(!out.contains_key("customXml/_rels/item1.xml.rels"));
    });
}

#[test]
fn custom_store_keeps_its_name_and_bibliography_takes_the_first_free_slot() {
    let mut doc = wordcraft_docx::read(&zip(&fixture())).unwrap();
    doc.sources.push(wordcraft_doc::Source { tag: "OriginalSource".into(), title: "Original synthetic reference".into(), ..Default::default() });
    let out = unzip(&wordcraft_docx::write(&doc).unwrap());
    // The bibliography is written to its own `customXml/item1.xml` (#532) and the store we
    // don't own moves aside under a fresh name, its data, properties and relationship kept.
    assert!(text(&out, "customXml/item1.xml").contains("<b:Tag>OriginalSource</b:Tag>"), "parts: {:?}", out.keys().collect::<Vec<_>>());
    let moved = out.iter().find(|(_, v)| String::from_utf8_lossy(v).contains("c:customer")).map(|(k, _)| k.as_str()).unwrap();
    assert_eq!(moved, "customXml/item1_2.xml", "the store keeps its data under a fresh name");
    assert_eq!(out.get("customXml/item1_2.xml").map(Vec::as_slice), Some(DATA.as_bytes()));
    assert_eq!(out.get("customXml/itemProps1_2.xml").map(Vec::as_slice), Some(PROPS.as_bytes()));
    assert!(text(&out, "customXml/_rels/item1_2.xml.rels").contains(r#"Target="itemProps1_2.xml""#));
    assert!(out.contains_key("customXml/itemProps1.xml"));
    let rels = text(&out, "word/_rels/document.xml.rels");
    assert!(rels.contains(r#"Target="../customXml/item1_2.xml""#), "the store keeps its relationship: {rels}");
    assert!(text(&out, "_rels/.rels").contains(r#"Target="customXml/item1.xml""#), "the bibliography is related from the package root");
    // Another save/read cycle keeps both where they are.
    let reread = wordcraft_docx::read(&wordcraft_docx::write(&doc).unwrap()).unwrap();
    assert_eq!(reread.sources.len(), 1);
    let out2 = unzip(&wordcraft_docx::write(&reread).unwrap());
    assert_eq!(out2.get("customXml/item1_2.xml").map(Vec::as_slice), Some(DATA.as_bytes()));
    assert!(text(&out2, "customXml/item1.xml").contains("<b:Tag>OriginalSource</b:Tag>"));
    assert!(!out2.contains_key("customXml/item1_3.xml"), "no slot churn");
}

#[test]
fn existing_bibliography_part_is_rewritten_in_place() {
    let mut p = fixture();
    let sources = r#"<b:Sources xmlns:b="http://schemas.openxmlformats.org/officeDocument/2006/bibliography"><b:Source><b:Tag>FromWord</b:Tag><b:SourceType>Book</b:SourceType><b:Author><b:Author><b:NameList><b:Person><b:Last>Rivera</b:Last><b:First>Alex</b:First></b:Person></b:NameList></b:Author></b:Author><b:Title>Shared Spaces</b:Title><b:Year>2021</b:Year></b:Source></b:Sources>"#;
    p.insert("customXml/item2.xml".into(), sources.as_bytes().to_vec());
    p.insert(
        "word/_rels/document.xml.rels".into(),
        rels(&[("data", "customXml", "../customXml/item1.xml", false), ("bib", "customXml", "../customXml/item2.xml", false)]).into_bytes(),
    );
    let mut doc = wordcraft_docx::read(&zip(&p)).unwrap();
    assert_eq!(doc.sources.len(), 1);
    assert_eq!(doc.sources[0].tag, "FromWord");
    assert_eq!(doc.sources[0].author, "Rivera, Alex");
    doc.sources.push(wordcraft_doc::Source { tag: "Added".into(), title: "Added later".into(), ..Default::default() });
    let out = unzip(&wordcraft_docx::write(&doc).unwrap());
    // The store the bibliography displaces keeps its data; the file's own part is not left
    // behind as a stale copy beside the rewritten one.
    assert_eq!(out.get("customXml/item1_2.xml").map(Vec::as_slice), Some(DATA.as_bytes()));
    assert_eq!(out.get("customXml/itemProps1_2.xml").map(Vec::as_slice), Some(PROPS.as_bytes()));
    let bib = text(&out, "customXml/item1.xml");
    assert!(bib.contains("<b:Tag>FromWord</b:Tag>") && bib.contains("<b:Tag>Added</b:Tag>"), "{bib}");
    assert!(!out.contains_key("customXml/item2.xml"), "no stale copy of the file's own part");
    assert!(out.contains_key("customXml/itemProps1.xml"));
    let rels = text(&out, "word/_rels/document.xml.rels");
    assert!(rels.contains(r#"Target="../customXml/item1_2.xml""#), "the store keeps its relationship: {rels}");
    assert!(text(&out, "_rels/.rels").contains(r#"Target="customXml/item1.xml""#), "the bibliography is related from the package root");
    // And it stays that way over another cycle.
    let reread = wordcraft_docx::read(&wordcraft_docx::write(&doc).unwrap()).unwrap();
    assert_eq!(reread.sources.len(), 2);
    assert_eq!(reread.sources[0].author, "Rivera, Alex");
    let out2 = unzip(&wordcraft_docx::write(&reread).unwrap());
    assert_eq!(out2.get("customXml/item1_2.xml").map(Vec::as_slice), Some(DATA.as_bytes()));
    assert!(out2.contains_key("customXml/item1.xml"));
    assert!(!out2.contains_key("customXml/item2.xml"));
}

#[test]
fn opaque_non_utf8_data_and_strict_relationship_types_are_preserved() {
    let mut p = fixture();
    let data: Vec<u8> =
        "<?xml version=\"1.0\" encoding=\"UTF-16\"?><customer>Atlas Studios</customer>".encode_utf16().flat_map(u16::to_le_bytes).collect();
    let mut bytes = vec![0xff, 0xfe];
    bytes.extend(data);
    p.insert("customXml/item1.xml".into(), bytes.clone());
    let strict =
        rels(&[("strict", "customXml", "../customXml/item1.xml", false)]).replace(RT, "http://purl.oclc.org/ooxml/officeDocument/relationships/");
    p.insert("word/_rels/document.xml.rels".into(), strict.into_bytes());
    edited_cycles(&p, |out| {
        assert_eq!(out.get("customXml/item1.xml"), Some(&bytes));
        assert!(text(out, "word/_rels/document.xml.rels").contains("http://purl.oclc.org/ooxml/officeDocument/relationships/customXml"));
    });
}

#[test]
fn reserved_part_targets_are_renamed_instead_of_shadowing_or_looping() {
    let mut p = fixture();
    p.insert("word/_rels/document.xml.rels".into(), rels(&[("hostile", "customXml", "../docProps/foreign.xml", false)]).into_bytes());
    p.insert("docProps/foreign.xml".into(), DATA.as_bytes().to_vec());
    edited_cycles(&p, |out| {
        assert!(out.values().any(|b| b == DATA.as_bytes()));
        assert!(!out.contains_key("docProps/foreign.xml"));
        assert!(text(out, "word/_rels/document.xml.rels").contains(r#"Target="embeddings/part.xml""#));
        assert!(text(out, "docProps/core.xml").contains("coreProperties"));
    });
}
