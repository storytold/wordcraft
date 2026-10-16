//! Original synthetic OOXML: inclusion/exclusion choices are opaque data, never merged here.
use std::collections::BTreeMap;
use std::io::{Cursor, Read, Write};
use std::sync::Arc;

const W: &str = "http://schemas.openxmlformats.org/wordprocessingml/2006/main";
const WNE: &str = "http://schemas.microsoft.com/office/word/2006/wordml";
const R: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships";
const WORD: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships/recipientData";
const ISO: &str = "http://schemas.openxmlformats.org/officeDocument/2006/mailMergeRecipientData";
const CT: &str = "application/vnd.openxmlformats-officedocument.wordprocessingml.mailMergeRecipientData+xml";
const MS_CT: &str = "application/vnd.ms-word.mailMergeRecipientData+xml";

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
        assert!(parts.insert(name, bytes).is_none(), "duplicate ZIP part");
    }
    parts
}

fn elements(bytes: &[u8], name: &[u8]) -> Vec<BTreeMap<String, String>> {
    let mut reader = quick_xml::Reader::from_reader(bytes);
    let mut matches = Vec::new();
    loop {
        match reader.read_event().unwrap() {
            quick_xml::events::Event::Start(e) | quick_xml::events::Event::Empty(e) if e.name().as_ref() == name => {
                matches.push(
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
            quick_xml::events::Event::Eof => return matches,
            _ => {}
        }
    }
}

fn change(parts: &mut BTreeMap<String, Vec<u8>>, part: &str, from: &str, to: &str) {
    let text = String::from_utf8(parts[part].clone()).unwrap();
    assert!(text.contains(from), "fixture replacement did not apply");
    parts.insert(part.into(), text.replace(from, to).into_bytes());
}

fn fixture(kind: &str, ct: &str, ns: &str) -> BTreeMap<String, Vec<u8>> {
    let payload = if ns == W {
        format!(
            r#"<v:recipients xmlns:v="{ns}"><v:recipientData><v:active v:val="0"/><v:column v:val="0"/><v:uniqueTag v:val="ZXhjbHVkZWQ="/></v:recipientData><v:recipientData><v:active v:val="1"/><v:column v:val="0"/><v:uniqueTag v:val="aW5jbHVkZWQ="/></v:recipientData></v:recipients>"#
        )
    } else {
        format!(
            r#"<v:recipients xmlns:v="{ns}"><v:recipientData><v:active v:val="0"/><v:hash v:val="101"/></v:recipientData><v:recipientData><v:active v:val="1"/><v:hash v:val="202"/></v:recipientData></v:recipients>"#
        )
    };
    [
        ("[Content_Types].xml", format!(r#"<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="xml" ContentType="application/xml"/><Override PartName="/word/document.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml"/><Override PartName="/word/settings.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.settings+xml"/><Override PartName="/word/recipientData.xml" ContentType="{ct}"/></Types>"#)),
        ("_rels/.rels", format!(r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="main" Type="{R}/officeDocument" Target="word/document.xml"/></Relationships>"#)),
        ("word/document.xml", format!(r#"<w:document xmlns:w="{W}"><w:body><w:p><w:fldSimple w:instr=" MERGEFIELD Name "><w:r><w:t>Cached Name</w:t></w:r></w:fldSimple></w:p></w:body></w:document>"#)),
        ("word/_rels/document.xml.rels", format!(r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="settings" Type="{R}/settings" Target="settings.xml"/></Relationships>"#)),
        ("word/settings.xml", format!(r#"<w:settings xmlns:w="{W}" xmlns:r="{R}"><w:attachedTemplate r:id="template"/><w:mailMerge><w:mainDocumentType w:val="formLetters"/><w:dataType w:val="spreadsheet"/><w:query w:val="SELECT Name FROM Letters"/><w:dataSource r:id="source"/><w:odso><w:src r:id="source"/><w:table w:val="Letters"/><w:recipientData r:id="rId1"/></w:odso></w:mailMerge></w:settings>"#)),
        ("word/_rels/settings.xml.rels", format!(r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="{kind}" Target="recipientData.xml" TargetMode="Internal"/><Relationship Id="source" Type="{R}/mailMergeSource" Target="file:///__missing_synthetic_source__/letters.xlsx" TargetMode="External"/><Relationship Id="template" Type="{R}/attachedTemplate" Target="file:///__missing_synthetic_source__/letter.dotx" TargetMode="External"/></Relationships>"#)),
        ("word/recipientData.xml", payload),
    ].into_iter().map(|(name, bytes)| (name.to_string(), bytes.into_bytes())).collect()
}

#[test]
fn embedded_recipient_choices_survive_edit_two_saves_and_native_json() {
    for (kind, ct, ns) in [(WORD, CT, W), (WORD, MS_CT, WNE), (ISO, CT, W)] {
        let input = fixture(kind, ct, ns);
        let mut doc = wordcraft_docx::read(&zip(&input)).unwrap();
        doc.body.push(wordcraft_doc::Block::Para(wordcraft_doc::Paragraph::with_text("Edited letter.", Default::default())).into());
        // The document's ordinary native serialization carries the same opaque store.
        doc = serde_json::from_slice(&serde_json::to_vec(&doc).unwrap()).unwrap();
        for _ in 0..2 {
            let bytes = wordcraft_docx::write(&doc).expect("embedded recipient choices must remain saveable");
            let output = unzip(&bytes);
            assert_eq!(output["word/recipientData.xml"], input["word/recipientData.xml"], "recipient flags and keys must be byte-identical");
            let types = String::from_utf8(output["[Content_Types].xml"].clone()).unwrap();
            assert!(types.contains(ct));
            let settings = String::from_utf8(output["word/settings.xml"].clone()).unwrap();
            assert!(settings.contains("w:recipientData") && settings.contains("SELECT Name FROM Letters"));
            let refs = elements(&output["word/settings.xml"], b"w:recipientData");
            assert_eq!(refs.len(), 1);
            let id = &refs[0]["r:id"];
            let rels = elements(&output["word/_rels/settings.xml.rels"], b"Relationship");
            let recipient = rels.iter().find(|r| &r["Id"] == id).unwrap();
            assert_eq!(recipient["Type"], kind);
            assert_eq!(recipient["Target"], "recipientData.xml");
            assert_ne!(recipient.get("TargetMode").map(String::as_str), Some("External"));
            assert_eq!(rels.len(), 3);
            let unique: std::collections::BTreeSet<_> = rels.iter().map(|r| &r["Id"]).collect();
            assert_eq!(unique.len(), rels.len(), "template/source/recipient IDs must not collide");
            doc = wordcraft_docx::read(&bytes).unwrap();
            assert!(doc.plain_text(wordcraft_doc::StoryRef::Body).contains("Edited letter."));
            assert!(String::from_utf8(output["word/document.xml"].clone()).unwrap().contains("MERGEFIELD Name"));
        }
    }
}

#[test]
fn unsupported_recipient_inputs_refuse_instead_of_dropping_choices() {
    let original = fixture(WORD, CT, W);
    let relpart = "word/_rels/settings.xml.rels";
    let mut cases = Vec::new();
    for (label, from, to) in [
        ("missing relationship", "Id=\"rId1\"", "Id=\"unused\""),
        ("external", "TargetMode=\"Internal\"", "TargetMode=\"External\""),
        ("unknown mode", "TargetMode=\"Internal\"", "TargetMode=\"Unknown\""),
        ("wrong type", WORD, "https://synthetic.invalid/recipientData"),
        ("empty target", "Target=\"recipientData.xml\"", "Target=\"\""),
        ("above root", "Target=\"recipientData.xml\"", "Target=\"../../word/recipientData.xml\""),
        ("fragment", "Target=\"recipientData.xml\"", "Target=\"recipientData.xml#other\""),
        ("encoded traversal", "Target=\"recipientData.xml\"", "Target=\"%2e%2e/recipientData.xml\""),
    ] {
        let mut input = original.clone();
        change(&mut input, relpart, from, to);
        cases.push((label, input));
    }
    for (label, part, bytes) in [
        ("wrong content type", "[Content_Types].xml", String::from_utf8(original["[Content_Types].xml"].clone()).unwrap().replace(CT, "application/xml").into_bytes()),
        ("wrong root", "word/recipientData.xml", format!(r#"<w:other xmlns:w="{W}"/>"#).into_bytes()),
        ("malformed part", "word/recipientData.xml", b"<unclosed".to_vec()),
        ("over limit", "word/recipientData.xml", vec![b' '; 1024 * 1024 + 1]),
        ("dependent relationships", "word/_rels/recipientData.xml.rels", format!(r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="child" Type="{R}/image" Target="missing.png"/></Relationships>"#).into_bytes()),
    ] {
        let mut input = original.clone();
        input.insert(part.into(), bytes);
        cases.push((label, input));
    }
    let mut missing = original.clone();
    missing.remove("word/recipientData.xml");
    cases.push(("missing part", missing));
    let mut duplicate = original.clone();
    change(
        &mut duplicate,
        relpart,
        "</Relationships>",
        &format!(r#"<Relationship Id="rId1" Type="{WORD}" Target="recipientData.xml"/></Relationships>"#),
    );
    cases.push(("duplicate id", duplicate));
    let mut multiple = original.clone();
    change(&mut multiple, "word/settings.xml", "</w:odso>", r#"<w:recipientData r:id="rId1"/></w:odso>"#);
    cases.push(("multiple recipient references", multiple));
    for (case, input) in cases {
        let doc = wordcraft_docx::read(&zip(&input)).unwrap();
        let err = wordcraft_docx::write(&doc).expect_err(case);
        assert!(err.to_string().contains("mail merge"), "{case}: {err}");
    }
}

#[test]
fn recipient_paths_follow_settings_and_writer_name_allocation() {
    for (part, target) in [("merge/choices.xml", "../merge/choices.xml"), ("word/header1.xml", "header1.xml")] {
        let mut input = fixture(WORD, CT, W);
        let original = input.remove("word/recipientData.xml").unwrap();
        input.insert(part.into(), original.clone());
        change(&mut input, "[Content_Types].xml", "/word/recipientData.xml", &format!("/{part}"));
        change(&mut input, "word/_rels/settings.xml.rels", "Target=\"recipientData.xml\" TargetMode=\"Internal\"", &format!("Target=\"{target}\""));
        input.insert("word/unusedRecipients.xml".into(), b"never copy an unreferenced part".to_vec());
        let output = unzip(&wordcraft_docx::write(&wordcraft_docx::read(&zip(&input)).unwrap()).unwrap());
        assert!(!output.contains_key("word/unusedRecipients.xml"));
        let rels = elements(&output["word/_rels/settings.xml.rels"], b"Relationship");
        let rel = rels.iter().find(|r| r["Type"] == WORD).unwrap();
        let written =
            if rel["Target"].starts_with("../") { rel["Target"].strip_prefix("../").unwrap().to_string() } else { format!("word/{}", rel["Target"]) };
        assert_eq!(output[&written], original);
        assert!(
            elements(&output["[Content_Types].xml"], b"Override").iter().any(|c| c["PartName"] == format!("/{written}") && c["ContentType"] == CT)
        );
        if part == "word/header1.xml" {
            assert_ne!(written, part, "reserved output names must be allocated elsewhere");
        }
    }
}

#[test]
fn losing_kept_recipient_bytes_refuses_the_save() {
    let mut doc = wordcraft_docx::read(&zip(&fixture(WORD, CT, W))).unwrap();
    doc.passthrough.remove("word/recipientData.xml");
    assert!(wordcraft_docx::write(&doc).unwrap_err().to_string().contains("mail merge"));
    doc.passthrough.insert("word/recipientData.xml".into(), Arc::new(vec![b' '; 1024 * 1024 + 1]));
    assert!(wordcraft_docx::write(&doc).unwrap_err().to_string().contains("mail merge"));
}
