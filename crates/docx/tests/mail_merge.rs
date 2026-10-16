//! Original synthetic OOXML: preserve a template's source metadata without opening its source.
use std::collections::BTreeMap;
use std::io::{Read, Write};

use quick_xml::events::Event;

const W: &str = "http://schemas.openxmlformats.org/wordprocessingml/2006/main";
const R: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships";
const SOURCE: &str = "file:///__wordcraft_missing_source__/recipients%20%26%20names.xlsx";
const ODSO: &str = "file:///__wordcraft_missing_source__/connection.odc";
const TEMPLATE: &str = "file:///__wordcraft_missing_source__/letter.dotx";

fn fixture(merge: &str, relationships: &str) -> Vec<u8> {
    let settings = format!(r#"<w:settings xmlns:w="{W}" xmlns:r="{R}"><w:attachedTemplate r:id="template"/>{merge}</w:settings>"#);
    let mut body = String::from("<w:p><w:r><w:t>Original letter.</w:t></w:r></w:p>");
    for field in ["Name", "Company", "Reference"] {
        body.push_str(&format!(r#"<w:p><w:fldSimple w:instr=" MERGEFIELD {field} "><w:r><w:t>Cached {field}</w:t></w:r></w:fldSimple></w:p>"#));
    }
    let document = format!(r#"<w:document xmlns:w="{W}" xmlns:r="{R}"><w:body>{body}</w:body></w:document>"#);
    let rels = format!(
        r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="template" Type="{R}/attachedTemplate" Target="{TEMPLATE}" TargetMode="External"/>{relationships}</Relationships>"#
    );
    let entries = [
        (
            "[Content_Types].xml",
            r#"<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="xml" ContentType="application/xml"/><Override PartName="/word/document.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml"/><Override PartName="/word/settings.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.settings+xml"/></Types>"#.to_string(),
        ),
        (
            "_rels/.rels",
            format!(
                r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="main" Type="{R}/officeDocument" Target="word/document.xml"/></Relationships>"#
            ),
        ),
        ("word/document.xml", document),
        (
            "word/_rels/document.xml.rels",
            format!(
                r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="settings" Type="{R}/settings" Target="settings.xml"/></Relationships>"#
            ),
        ),
        ("word/settings.xml", settings),
        ("word/_rels/settings.xml.rels", rels),
    ];
    let mut zip = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    for (name, xml) in entries {
        zip.start_file(name, zip::write::SimpleFileOptions::default()).unwrap();
        zip.write_all(xml.as_bytes()).unwrap();
    }
    zip.finish().unwrap().into_inner()
}

fn merge() -> String {
    r#"<w:mailMerge><w:mainDocumentType w:val="formLetters"/><w:dataType w:val="spreadsheet"/><w:connectString w:val="Data Source=synthetic-only"/><w:query w:val="SELECT Name, Company, Reference FROM Letters"/><w:dataSource r:id="rId1"/><w:odso><w:src r:id="rId2"/><w:table w:val="Letters"/><w:type w:val="database"/></w:odso></w:mailMerge>"#.into()
}

fn source_rels() -> String {
    format!(
        r#"<Relationship Id="rId1" Type="{R}/mailMergeSource" Target="{SOURCE}" TargetMode="External"/><Relationship Id="rId2" Type="{R}/mailMergeSource" Target="{ODSO}" TargetMode="External"/>"#
    )
}

fn part(bytes: &[u8], name: &str) -> String {
    let mut zip = zip::ZipArchive::new(std::io::Cursor::new(bytes)).unwrap();
    let mut text = String::new();
    zip.by_name(name).unwrap().read_to_string(&mut text).unwrap();
    text
}

fn elements(xml: &str, name: &str) -> Vec<BTreeMap<String, String>> {
    let mut reader = quick_xml::Reader::from_str(xml);
    let mut found = Vec::new();
    loop {
        match reader.read_event().unwrap() {
            Event::Start(e) | Event::Empty(e) if e.name().as_ref() == name.as_bytes() => {
                found.push(
                    e.attributes()
                        .map(|a| {
                            let a = a.unwrap();
                            (
                                String::from_utf8(a.key.as_ref().to_vec()).unwrap(),
                                a.normalized_value(quick_xml::XmlVersion::Explicit1_0).unwrap().into_owned(),
                            )
                        })
                        .collect(),
                );
            }
            Event::Eof => return found,
            _ => {}
        }
    }
}

fn assert_sources(bytes: &[u8]) {
    let settings = part(bytes, "word/settings.xml");
    assert_eq!(elements(&settings, "w:mailMerge").len(), 1, "mail-merge identity lost: {settings}");
    assert_eq!(elements(&settings, "w:mainDocumentType")[0]["w:val"], "formLetters");
    assert_eq!(elements(&settings, "w:query")[0]["w:val"], "SELECT Name, Company, Reference FROM Letters");
    assert_eq!(elements(&settings, "w:connectString")[0]["w:val"], "Data Source=synthetic-only");
    assert_eq!(elements(&settings, "w:dataType")[0]["w:val"], "spreadsheet");
    assert_eq!(elements(&settings, "w:table")[0]["w:val"], "Letters");
    let rels = elements(&part(bytes, "word/_rels/settings.xml.rels"), "Relationship");
    let ids: std::collections::BTreeSet<_> = rels.iter().map(|r| &r["Id"]).collect();
    assert_eq!(ids.len(), rels.len(), "duplicate relationship IDs");
    assert_eq!(rels.len(), 3, "only referenced sources and attached template");
    for (tag, target, kind) in
        [("w:dataSource", SOURCE, "mailMergeSource"), ("w:src", ODSO, "mailMergeSource"), ("w:attachedTemplate", TEMPLATE, "attachedTemplate")]
    {
        let refs = elements(&settings, tag);
        assert_eq!(refs.len(), 1);
        let id = &refs[0]["r:id"];
        let rel = rels.iter().find(|r| &r["Id"] == id).unwrap_or_else(|| panic!("dangling {tag}: {id}"));
        assert_eq!(rel["Type"], format!("{R}/{kind}"));
        assert_eq!(rel["Target"], target);
        assert_eq!(rel["TargetMode"], "External");
    }
    let body = part(bytes, "word/document.xml");
    for field in ["Name", "Company", "Reference"] {
        assert!(body.contains(&format!("MERGEFIELD {field}")), "field instruction lost: {body}");
    }
}

#[test]
fn mail_merge_sources_survive_edit_and_two_docx_saves() {
    let original = fixture(&merge(), &source_rels());
    assert_sources(&original);
    let mut doc = wordcraft_docx::read(&original).unwrap();
    doc.body.push(wordcraft_doc::Block::Para(wordcraft_doc::Paragraph::with_text("Local edit retained.", Default::default())).into());
    for _ in 0..2 {
        let saved = wordcraft_docx::write(&doc).unwrap();
        assert_sources(&saved);
        doc = wordcraft_docx::read(&saved).unwrap();
        assert!(doc.plain_text(wordcraft_doc::StoryRef::Body).contains("Local edit retained."));
    }
}

#[test]
fn unusable_mail_merge_sources_refuse_the_save() {
    let valid = source_rels();
    let cases = [
        ("missing", valid.replace("Id=\"rId2\"", "Id=\"unused\"")),
        ("wrong kind", valid.replace("/mailMergeSource", "/image")),
        ("internal", valid.replace("TargetMode=\"External\"", "")),
        ("empty target", valid.replace(ODSO, "")),
        ("duplicate", format!("{valid}{valid}")),
        ("malformed", "<Relationship".to_string()),
    ];
    for (case, rels) in cases {
        let doc = wordcraft_docx::read(&fixture(&merge(), &rels)).unwrap();
        let err = wordcraft_docx::write(&doc).expect_err(case);
        assert!(err.to_string().contains("mail merge"), "{case}: {err}");
        assert!(doc.plain_text(wordcraft_doc::StoryRef::Body).contains("Original letter."));
    }
}

#[test]
fn unrelated_relationships_are_not_carried_and_shared_sources_reuse_an_id() {
    let rels =
        format!(r#"{}<Relationship Id="unused" Type="{R}/image" Target="https://example.invalid/unrelated" TargetMode="External"/>"#, source_rels());
    let doc = wordcraft_docx::read(&fixture(&merge(), &rels)).unwrap();
    assert_sources(&wordcraft_docx::write(&doc).unwrap());
    let doc = wordcraft_docx::read(&fixture(&merge().replace("rId2", "rId1"), &rels)).unwrap();
    let saved = wordcraft_docx::write(&doc).unwrap();
    let settings = part(&saved, "word/settings.xml");
    assert_eq!(elements(&settings, "w:dataSource")[0]["r:id"], elements(&settings, "w:src")[0]["r:id"]);
    let rels = elements(&part(&saved, "word/_rels/settings.xml.rels"), "Relationship");
    assert_eq!(rels.len(), 2);
    assert!(!rels.iter().any(|r| r["Target"] == ODSO || r["Target"].contains("unrelated")));
}

#[test]
fn header_sources_and_prefix_aliases_survive() {
    let merge = merge().replace("<w:odso>", "<w:headerSource r:id=\"header\"/><w:odso>");
    let merge = merge
        .replace("<w:mailMerge>", &format!(r#"<w:mailMerge xmlns:p="{W}" xmlns:rel="{R}">"#))
        .replace("<w:src r:id=\"rId2\"/>", "<p:src rel:id='rId2'/>");
    let rels = format!(
        r#"{}<Relationship Id="header" Type="{R}/mailMergeHeaderSource" Target="file:///__wordcraft_missing_source__/columns.csv" TargetMode="External"/>"#,
        source_rels()
    );
    let mut doc = wordcraft_docx::read(&fixture(&merge, &rels)).unwrap();
    doc.settings.link_styles = true;
    for _ in 0..2 {
        let saved = wordcraft_docx::write(&doc).unwrap();
        let settings = part(&saved, "word/settings.xml");
        assert!(settings.find("w:linkStyles").unwrap() < settings.find("<w:mailMerge").unwrap());
        let header_id = &elements(&settings, "w:headerSource")[0]["r:id"];
        let rels = elements(&part(&saved, "word/_rels/settings.xml.rels"), "Relationship");
        let header = rels.iter().find(|r| &r["Id"] == header_id).unwrap();
        assert_eq!(header["Type"], format!("{R}/mailMergeHeaderSource"));
        assert_eq!(header["Target"], "file:///__wordcraft_missing_source__/columns.csv");
        assert_eq!(header["TargetMode"], "External");
        let src_id = &elements(&settings, "w:src")[0]["r:id"];
        assert_eq!(rels.iter().find(|r| &r["Id"] == src_id).unwrap()["Target"], ODSO);
        doc = wordcraft_docx::read(&saved).unwrap();
    }
}

#[test]
fn unrecognised_mail_merge_markup_is_not_silently_dropped() {
    let merge = merge().replace("</w:mailMerge>", "<x:extra xmlns:x=\"urn:synthetic:unsupported\"/></w:mailMerge>");
    let doc = wordcraft_docx::read(&fixture(&merge, &source_rels())).unwrap();
    assert!(wordcraft_docx::write(&doc).unwrap_err().to_string().contains("unrecognised namespace"));
}
