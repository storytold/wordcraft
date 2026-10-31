//! Original synthetic OOXML: inclusion/exclusion choices are opaque data, never merged here.
use std::collections::BTreeMap;
use std::io::{Cursor, Read, Write};
use std::sync::Arc;

const W: &str = "http://schemas.openxmlformats.org/wordprocessingml/2006/main";
const WNE: &str = "http://schemas.microsoft.com/office/word/2006/wordml";
const R: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships";
const WORD: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships/recipientData";
const ISO: &str = "http://schemas.openxmlformats.org/officeDocument/2006/mailMergeRecipientData";
const CANONICAL: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships/mailMergeRecipientData";
const STRICT: &str = "http://purl.oclc.org/ooxml/officeDocument/relationships/mailMergeRecipientData";
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
            assert!(
                elements(&output["[Content_Types].xml"], b"Override")
                    .iter()
                    .any(|o| o["PartName"] == "/word/recipientData.xml" && o["ContentType"] == ct),
                "the emitted recipient part must retain its own content type"
            );
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
fn recipient_relationships_follow_emitted_settings_through_aliases_and_native_reopen() {
    const SW: &str = "http://purl.oclc.org/ooxml/wordprocessingml/main";
    const SR: &str = "http://purl.oclc.org/ooxml/officeDocument/relationships";
    for kind in [CANONICAL, STRICT, WORD, ISO] {
        for strict in [false, true] {
            for alias in [false, true] {
                let mut input = fixture("urn:original:recipient", CT, W);
                for (name, bytes) in &mut input {
                    let mut text = String::from_utf8(bytes.clone()).unwrap();
                    if strict {
                        text = text.replace(W, SW).replace(R, SR);
                    }
                    if name == "word/settings.xml" && alias {
                        text = text.replace("w:", "x:").replace("xmlns:w=", "xmlns:x=").replace("r:", "rel:").replace("xmlns:r=", "xmlns:rel=");
                    }
                    *bytes = text.replace("urn:original:recipient", kind).into_bytes();
                }
                let mut doc = wordcraft_docx::read(&zip(&input)).unwrap();
                doc.body.push(wordcraft_doc::Block::Para(wordcraft_doc::Paragraph::with_text("Edited letter.", Default::default())).into());
                for _ in 0..2 {
                    doc = serde_json::from_slice(&serde_json::to_vec(&doc).unwrap()).unwrap();
                    let output = unzip(&wordcraft_docx::write(&doc).unwrap());
                    assert_eq!(output["word/recipientData.xml"], input["word/recipientData.xml"]);
                    let settings = std::str::from_utf8(&output["word/settings.xml"]).unwrap();
                    let settings = roxmltree::Document::parse(settings).unwrap();
                    let (wn, rn) = if strict && !alias { (SW, SR) } else { (W, R) };
                    assert!(settings.root_element().has_tag_name((wn, "settings")));
                    let reference = settings.descendants().find(|n| n.has_tag_name((wn, "recipientData"))).unwrap();
                    let id = reference.attribute((rn, "id")).unwrap();
                    let rels = elements(&output["word/_rels/settings.xml.rels"], b"Relationship");
                    let recipient = rels.iter().find(|r| r["Id"] == id).unwrap();
                    let expected = if [CANONICAL, STRICT].contains(&kind) { format!("{rn}/mailMergeRecipientData") } else { kind.into() };
                    assert_eq!(recipient["Type"], expected, "input={kind}, strict={strict}, alias={alias}");
                    assert_ne!(recipient.get("TargetMode").map(String::as_str), Some("External"));
                    assert_eq!(recipient["Target"], "recipientData.xml");
                    assert!(
                        elements(&output["[Content_Types].xml"], b"Override")
                            .iter()
                            .any(|o| o["PartName"] == "/word/recipientData.xml" && o["ContentType"] == CT)
                    );
                    doc = wordcraft_docx::read(&zip(&output)).unwrap();
                    assert!(doc.plain_text(wordcraft_doc::StoryRef::Body).contains("Edited letter."));
                }
            }
        }
    }
}

#[test]
fn native_recipient_modes_match_import_acceptance() {
    for kind in [WORD, ISO, CANONICAL, STRICT] {
        let original = fixture(kind, CT, W);
        let native = wordcraft_docx::read(&zip(&original)).unwrap();
        let key = "wordcraft:settings.mailMerge.rels";
        let rels = std::str::from_utf8(&native.passthrough[key]).unwrap();
        for (mode, allowed) in [
            (None, true),
            (Some("Internal"), true),
            (Some("iNtErNaL"), true),
            (Some("Unknown"), false),
            (Some(""), false),
            (Some(" Internal"), false),
            (Some("Internal "), false),
            (Some("External"), false),
            (Some("EXTERNAL"), false),
            (Some("External "), false),
        ] {
            let mut doc = native.clone();
            let changed = mode.map_or_else(|| rels.to_owned(), |m| rels.replacen("Id=\"rId1\"", &format!("Id=\"rId1\" TargetMode=\"{m}\""), 1));
            doc.passthrough.insert(key.into(), Arc::new(changed.into_bytes()));
            doc = serde_json::from_slice(&serde_json::to_vec(&doc).unwrap()).unwrap();
            let result = wordcraft_docx::write(&doc);
            if allowed {
                let saved = result.unwrap();
                assert_eq!(unzip(&saved)["word/recipientData.xml"], original["word/recipientData.xml"]);
                let reopened = wordcraft_docx::read(&saved).unwrap();
                assert!(wordcraft_docx::write(&reopened).is_ok());
            } else {
                assert!(result.unwrap_err().to_string().contains("mail merge"), "{kind}: {mode:?}");
            }
        }
    }
}

#[test]
fn structurally_malformed_recipient_xml_refuses_docx_and_native_saves() {
    for (kind, ct, ns) in [(WORD, CT, W), (WORD, MS_CT, WNE), (ISO, CT, W)] {
        let original = fixture(kind, ct, ns);
        let valid = String::from_utf8(original["word/recipientData.xml"].clone()).unwrap();
        let malformed = [
            valid.replace("</v:recipients>", ""),
            valid.replace("</v:recipientData>", ""),
            valid.replace("</v:recipients>", "</v:other>"),
            valid.replace("</v:recipientData>", "</v:other>"),
            format!("{valid}<extra/>"),
            format!("{valid}<extra>"),
            valid.replace("</v:recipients>", "&undefined;</v:recipients>"),
            format!("{valid}garbage"),
            format!("garbage{valid}"),
            valid.replacen("v:val=\"0\"", "v:val=\"0\" v:val=\"1\"", 1),
            valid.replacen("v:val=\"0\"", "v:val=\"&undefined;\"", 1),
            valid.replacen("v:val=\"0\"", "v:val=\"a&b\"", 1),
            valid.replacen("v:val=\"0\"", "v:val=0", 1),
            format!("{valid}<![CDATA[outside]]>"),
            format!("{valid}&amp;"),
            format!("{valid}<!-- bad--comment -->"),
            format!("{valid}<?xml version=\"1.0\"?>"),
            valid.replace("</v:recipients>", "<?xml version=\"1.0\"?></v:recipients>"),
        ];
        for payload in malformed {
            let mut input = original.clone();
            input.insert("word/recipientData.xml".into(), payload.as_bytes().to_vec());
            let imported = wordcraft_docx::read(&zip(&input)).unwrap();
            assert!(wordcraft_docx::write(&imported).expect_err(&payload).to_string().contains("mail merge"));
            let mut native = wordcraft_docx::read(&zip(&original)).unwrap();
            native.passthrough.insert("word/recipientData.xml".into(), Arc::new(payload.into_bytes()));
            native = serde_json::from_slice(&serde_json::to_vec(&native).unwrap()).unwrap();
            assert!(wordcraft_docx::write(&native).unwrap_err().to_string().contains("mail merge"));
        }
        // Legal surrounding markup and whitespace stay byte-identical, not regenerated.
        let decorated = format!("<?xml version=\"1.0\"?>\n<!-- original -->{valid}\n<?preserve original?>").into_bytes();
        let mut input = original;
        input.insert("word/recipientData.xml".into(), decorated.clone());
        let output = unzip(&wordcraft_docx::write(&wordcraft_docx::read(&zip(&input)).unwrap()).unwrap());
        assert_eq!(output["word/recipientData.xml"], decorated);
    }
}

#[test]
fn recipient_namespaces_and_encoding_are_validated_before_opaque_copying() {
    for (kind, ct, ns) in [(WORD, CT, W), (WORD, MS_CT, WNE), (ISO, CT, W)] {
        let original = fixture(kind, ct, ns);
        let valid = String::from_utf8(original["word/recipientData.xml"].clone()).unwrap();
        let mut odd_utf16 = vec![0xFF, 0xFE];
        odd_utf16.extend(valid.encode_utf16().flat_map(u16::to_le_bytes));
        odd_utf16.push(b'x');
        let malformed = [
            format!("{valid}<!DOCTYPE recipients>").into_bytes(),
            valid.replace("</v:recipients>", "<!DOCTYPE recipients></v:recipients>").into_bytes(),
            valid.replace("</v:recipients>", "<missing:choice/></v:recipients>").into_bytes(),
            valid.replacen("v:val=\"0\"", "v:val=\"0\" missing:choice=\"1\"", 1).into_bytes(),
            valid.replacen("v:val=\"0\"", &format!("v:val=\"0\" xmlns:z=\"{ns}\" z:val=\"1\""), 1).into_bytes(),
            [valid.strip_suffix("</v:recipients>").unwrap().as_bytes(), b"<![CDATA[\xff]]></v:recipients>"].concat(),
            [valid.as_bytes(), b"<!-- \xff -->"].concat(),
            odd_utf16,
        ];
        for (i, payload) in malformed.into_iter().enumerate() {
            let mut input = original.clone();
            input.insert("word/recipientData.xml".into(), payload.clone());
            let imported = wordcraft_docx::read(&zip(&input)).unwrap();
            assert!(wordcraft_docx::write(&imported).unwrap_err().to_string().contains("mail merge"), "case {i}");
            let mut native = wordcraft_docx::read(&zip(&original)).unwrap();
            native.passthrough.insert("word/recipientData.xml".into(), Arc::new(payload));
            native = serde_json::from_slice(&serde_json::to_vec(&native).unwrap()).unwrap();
            assert!(wordcraft_docx::write(&native).unwrap_err().to_string().contains("mail merge"), "native case {i}");
        }
        // Valid namespace aliases, CDATA, Unicode and UTF-16 retain their original bytes.
        let decorated = valid.replace("</v:recipients>", "<![CDATA[Choice α]]></v:recipients>");
        for payload in [
            [b"\xef\xbb\xbf".as_slice(), decorated.as_bytes()].concat(),
            [vec![0xFF, 0xFE], decorated.encode_utf16().flat_map(u16::to_le_bytes).collect()].concat(),
            [vec![0xFE, 0xFF], decorated.encode_utf16().flat_map(u16::to_be_bytes).collect()].concat(),
        ] {
            let mut input = original.clone();
            input.insert("word/recipientData.xml".into(), payload.clone());
            let mut doc = wordcraft_docx::read(&zip(&input)).unwrap();
            for _ in 0..2 {
                let saved = wordcraft_docx::write(&doc).unwrap();
                assert_eq!(unzip(&saved)["word/recipientData.xml"], payload);
                doc = serde_json::from_slice(&serde_json::to_vec(&wordcraft_docx::read(&saved).unwrap()).unwrap()).unwrap();
            }
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

#[test]
fn recipient_part_spelling_follows_package_canonicalization() {
    for (stored, target) in [
        ("word/RecipientData.xml", "recipientData.xml"),
        ("word/recipientData.xml", "RecipientData.xml"),
        ("word/RECIPIENTDATA.XML", "ReCiPiEnTdAtA.xMl"),
    ] {
        let mut input = fixture(WORD, CT, W);
        let payload = input.remove("word/recipientData.xml").unwrap();
        input.insert(stored.into(), payload.clone());
        change(&mut input, "[Content_Types].xml", "/word/recipientData.xml", &format!("/{stored}"));
        change(&mut input, "word/_rels/settings.xml.rels", "Target=\"recipientData.xml\"", &format!("Target=\"{target}\""));
        let mut doc = wordcraft_docx::read(&zip(&input)).unwrap();
        doc = serde_json::from_slice(&serde_json::to_vec(&doc).unwrap()).unwrap();
        for _ in 0..2 {
            let saved = wordcraft_docx::write(&doc).expect("a case-insensitively resolved recipient part must stay saveable");
            let output = unzip(&saved);
            assert_eq!(output[stored], payload, "recipient flags and keys must stay byte-identical");
            let refs = elements(&output["word/settings.xml"], b"w:recipientData");
            assert_eq!(refs.len(), 1);
            let rels = elements(&output["word/_rels/settings.xml.rels"], b"Relationship");
            let rel = rels.iter().find(|r| r["Id"] == refs[0]["r:id"]).unwrap();
            assert_eq!(format!("word/{}", rel["Target"]), stored);
            assert!(
                elements(&output["[Content_Types].xml"], b"Override").iter().any(|c| c["PartName"] == format!("/{stored}") && c["ContentType"] == CT)
            );
            assert!(String::from_utf8(output["word/settings.xml"].clone()).unwrap().contains("SELECT Name FROM Letters"));
            doc = wordcraft_docx::read(&saved).unwrap();
        }
    }
}

#[test]
fn recipient_case_collisions_refuse_only_when_both_spellings_are_referenced() {
    for recipient in ["recipientData.xml", "RecipientData.xml"] {
        for other_owner in ["document", "settings"] {
            for both_referenced in [false, true] {
                let mut input = fixture(WORD, CT, W);
                let lower = input["word/recipientData.xml"].clone();
                let upper = String::from_utf8(lower.clone()).unwrap().replace("ZXhjbHVkZWQ=", "b3RoZXIta2V5").into_bytes();
                assert_ne!(lower, upper);
                input.insert("word/RecipientData.xml".into(), upper.clone());
                change(
                    &mut input,
                    "[Content_Types].xml",
                    "</Types>",
                    &format!(r#"<Override PartName="/word/RecipientData.xml" ContentType="{CT}"/></Types>"#),
                );
                change(&mut input, "word/_rels/settings.xml.rels", "Target=\"recipientData.xml\"", &format!("Target=\"{recipient}\""));
                if both_referenced {
                    let other = if recipient == "recipientData.xml" { "RecipientData.xml" } else { "recipientData.xml" };
                    change(
                        &mut input,
                        &format!("word/_rels/{other_owner}.xml.rels"),
                        "</Relationships>",
                        &format!(r#"<Relationship Id="unused" Type="https://synthetic.invalid/unused" Target="{other}"/></Relationships>"#),
                    );
                }
                let mut doc = wordcraft_docx::read(&zip(&input)).unwrap();
                doc = serde_json::from_slice(&serde_json::to_vec(&doc).unwrap()).unwrap();
                for _ in 0..2 {
                    if both_referenced {
                        let err = wordcraft_docx::write(&doc).expect_err("two relationship-named spellings are ambiguous");
                        assert!(err.to_string().contains("mail merge"), "{err}");
                        break;
                    }
                    let saved = wordcraft_docx::write(&doc).expect("only the recipient's spelling is referenced");
                    let output = unzip(&saved);
                    let stored = format!("word/{recipient}");
                    assert_eq!(output[&stored], if recipient == "recipientData.xml" { &lower } else { &upper }.clone());
                    assert!(!output.contains_key(if recipient == "recipientData.xml" { "word/RecipientData.xml" } else { "word/recipientData.xml" }));
                    doc = wordcraft_docx::read(&saved).unwrap();
                }
            }
        }
    }
}

#[test]
fn recipient_structural_budgets_cover_attributes_namespaces_and_nesting() {
    let original = fixture(WORD, CT, W);
    for (count, namespaces, nested) in [(1023, false, false), (1024, false, false), (1024, true, false), (90_000, false, false), (1024, false, true)]
    {
        let attrs =
            (0..count).map(|i| if namespaces { format!(" xmlns:n{i}=\"urn:original:{i}\"") } else { format!(" a{i}=\"\"") }).collect::<String>();
        let payload = if nested {
            format!("<v:recipients xmlns:v=\"{W}\"><v:choice{attrs}></v:choice></v:recipients>")
        } else {
            format!("<v:recipients xmlns:v=\"{W}\"{attrs}/>")
        }
        .into_bytes();
        assert!(payload.len() < 1024 * 1024);
        let mut input = original.clone();
        input.insert("word/recipientData.xml".into(), payload.clone());
        let imported = wordcraft_docx::read(&zip(&input)).unwrap();
        let mut native = wordcraft_docx::read(&zip(&original)).unwrap();
        native.passthrough.insert("word/recipientData.xml".into(), Arc::new(payload.clone()));
        for doc in [imported, serde_json::from_slice(&serde_json::to_vec(&native).unwrap()).unwrap()] {
            if count == 1023 || nested {
                let output = unzip(&wordcraft_docx::write(&doc).unwrap());
                assert_eq!(output["word/recipientData.xml"], payload);
            } else {
                assert!(wordcraft_docx::write(&doc).unwrap_err().to_string().contains("mail merge"));
            }
        }
    }
    for depth in [256, 257] {
        let payload =
            format!("<v:recipients xmlns:v=\"{W}\">{}{}{}", "<v:choice>".repeat(depth - 1), "</v:choice>".repeat(depth - 1), "</v:recipients>")
                .into_bytes();
        let mut input = original.clone();
        input.insert("word/recipientData.xml".into(), payload.clone());
        let doc = wordcraft_docx::read(&zip(&input)).unwrap();
        if depth == 256 {
            assert_eq!(unzip(&wordcraft_docx::write(&doc).unwrap())["word/recipientData.xml"], payload);
        } else {
            assert!(wordcraft_docx::write(&doc).unwrap_err().to_string().contains("mail merge"));
        }
    }
}

#[test]
fn recipient_encoding_declarations_match_original_docx_and_native_bytes() {
    for (kind, ct, ns) in [(WORD, CT, W), (WORD, MS_CT, WNE), (ISO, CT, W)] {
        let original = fixture(kind, ct, ns);
        for (wire, declared, accepted) in [
            ("utf8", "UTF-8", true),
            ("utf8", "utf-8", true),
            ("utf8-bom", "Utf-8", true),
            ("utf16-le", "UTF-16", true),
            ("utf16-be", "UTF-16", true),
            ("utf16-le", "UTF-16LE", false),
            ("utf16-be", "UTF-16BE", false),
            ("utf16-le", "utf-16", true),
            ("utf16-be", "utf-16", true),
            ("utf8", "UTF-16", false),
            ("utf8-bom", "UTF-16", false),
            ("utf16-le", "UTF-8", false),
            ("utf16-be", "UTF-8", false),
            ("utf16-le", "UTF-16BE", false),
            ("utf16-be", "UTF-16LE", false),
            ("utf8", "ISO-8859-1", false),
            ("utf8", "x-original", false),
        ] {
            let text = format!("<?xml version='1.0' encoding='{declared}'?><v:recipients xmlns:v='{ns}'/>");
            let payload = match wire {
                "utf16-le" => [vec![0xFF, 0xFE], text.encode_utf16().flat_map(u16::to_le_bytes).collect()].concat(),
                "utf16-be" => [vec![0xFE, 0xFF], text.encode_utf16().flat_map(u16::to_be_bytes).collect()].concat(),
                "utf8-bom" => [b"\xef\xbb\xbf".as_slice(), text.as_bytes()].concat(),
                _ => text.into_bytes(),
            };
            let mut input = original.clone();
            input.insert("word/recipientData.xml".into(), payload.clone());
            let imported = wordcraft_docx::read(&zip(&input)).unwrap();
            let mut native = wordcraft_docx::read(&zip(&original)).unwrap();
            native.passthrough.insert("word/recipientData.xml".into(), Arc::new(payload.clone()));
            for mut doc in [imported, serde_json::from_slice(&serde_json::to_vec(&native).unwrap()).unwrap()] {
                for _ in 0..2 {
                    if accepted {
                        let saved = wordcraft_docx::write(&doc).unwrap();
                        assert_eq!(unzip(&saved)["word/recipientData.xml"], payload);
                        doc = serde_json::from_slice(&serde_json::to_vec(&wordcraft_docx::read(&saved).unwrap()).unwrap()).unwrap();
                    } else {
                        assert!(wordcraft_docx::write(&doc).unwrap_err().to_string().contains("mail merge"), "{wire}/{declared}");
                        break;
                    }
                }
            }
        }
    }
}
