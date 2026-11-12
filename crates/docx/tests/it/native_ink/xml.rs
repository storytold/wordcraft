use super::*;

/// Replaces the package's InkML part while preserving all other DOCX entries.
fn replace_xml(bytes: &[u8], ink: &[u8]) -> Vec<u8> {
    let mut files = unzip(bytes);
    files.insert("word/ink/ink1.xml".into(), ink.to_vec());
    let entries: Vec<_> = files.iter().map(|(path, bytes)| (path.as_str(), bytes.as_slice())).collect();
    zip(&entries)
}

/// Encodes XML as BOM-marked UTF-16 in the requested byte order.
fn utf16(text: &str, little: bool) -> Vec<u8> {
    let mut bytes = if little { vec![0xff, 0xfe] } else { vec![0xfe, 0xff] };
    for unit in text.encode_utf16() {
        bytes.extend(if little { unit.to_le_bytes() } else { unit.to_be_bytes() });
    }
    bytes
}

/// Supported XML versions and UTF encodings preserve InkML vectors and exact source bytes.
#[test]
fn native_ink_xml_versions_and_encodings_preserve_vectors_and_original_bytes() {
    let text = format!("<?xml version=\"1.1\" encoding=\"UTF-8\"?>{}", INK_XML.replace("/InkML", "/Ink&#77;L").replace("<trace ", "<trace\u{85}"));
    let mut variants = vec![text.as_bytes().to_vec()];
    for little in [false, true] {
        variants.push(utf16(&text.replace("UTF-8", "UTF-16"), little));
    }
    for ink in variants {
        for fallback in [false, true] {
            for (bytes, grouped) in frames(fallback) {
                let mut doc = wordcraft_docx::read(&replace_xml(&bytes, &ink)).unwrap();
                let original = ink_graphic(&wordcraft_docx::read(&bytes).unwrap(), grouped).items.clone();
                assert!(original.iter().any(|item| matches!(item, GraphicItem::Path { .. })));
                doc = serde_json::from_slice(&serde_json::to_vec(&doc).unwrap()).unwrap();
                for _ in 0..3 {
                    assert_eq!(ink_graphic(&doc, grouped).items, original);
                    let saved = wordcraft_docx::write(&doc).unwrap();
                    let files = unzip(&saved);
                    assert!(files.values().any(|bytes| bytes == &ink));
                    if fallback {
                        assert!(files.values().any(|bytes| bytes == b"original preview bytes"));
                    }
                    doc = wordcraft_docx::read(&saved).unwrap();
                }
            }
        }
    }
}

/// Bypassing the unrelated bibliography parser must not hide requirements in a typed InkML part.
#[test]
fn native_ink_markup_compatibility_requirements_remain_fatal() {
    let ink = INK_XML.replacen(
        "<ink ",
        r#"<ink xmlns:mc="http://schemas.openxmlformats.org/markup-compatibility/2006" xmlns:f="urn:unsupported" mc:MustUnderstand="f" "#,
        1,
    );
    for bytes in [ink.as_bytes().to_vec(), utf16(&ink, true), utf16(&ink, false)] {
        for fallback in [false, true] {
            for (docx, _) in frames(fallback) {
                assert!(matches!(wordcraft_docx::read(&replace_xml(&docx, &bytes)), Err(wordcraft_docx::DocxError::Xml(_))));
            }
        }
    }
    // A referenced part remains checked even when unrelated custom-XML relationships exhaust the
    // bibliography candidate limit before the ink relationship is reached.
    for (docx, _) in frames(true) {
        let mut files = unzip(&replace_xml(&docx, ink.as_bytes()));
        let extra: String = (0..64)
            .map(|i| format!(r#"<Relationship Id="other{i}" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/customXml" Target="unused{i}.xml"/>"#))
            .collect();
        let rels = text(&files, "word/_rels/document.xml.rels").replace(r#"<Relationship Id="rInk""#, &format!(r#"{extra}<Relationship Id="rInk""#));
        assert!(rels.contains(r#"Id="other63""#));
        files.insert("word/_rels/document.xml.rels".into(), rels.into_bytes());
        let entries: Vec<_> = files.iter().map(|(path, bytes)| (path.as_str(), bytes.as_slice())).collect();
        assert!(matches!(wordcraft_docx::read(&zip(&entries)), Err(wordcraft_docx::DocxError::Xml(_))));
    }
    // An unknown encoding could hide the directive from an ASCII byte scan; refuse it.
    let encoded = format!("<?xml version=\"1.0\" encoding=\"UTF-7\"?>{INK_XML}");
    assert!(matches!(wordcraft_docx::read(&replace_xml(&frames(true)[0].0, encoded.as_bytes())), Err(wordcraft_docx::DocxError::Xml(_))));
}

/// An unambiguously non-XML binary part may be replayed but must not be lost on edits.
#[test]
fn binary_native_ink_requires_unchanged_source_and_cannot_hide_mc() {
    let source = replace_xml(&frames(true)[0].0, &[0xff, 0x00, b'<', 0xfe]);
    let mut doc = wordcraft_docx::read(&source).unwrap();
    let saved = wordcraft_docx::write(&doc).unwrap();
    assert!(unzip(&saved).values().any(|b| b == &[0xff, 0x00, b'<', 0xfe]));
    doc.core.title = "changed".into();
    assert!(matches!(wordcraft_docx::write(&doc), Err(wordcraft_docx::DocxError::Preservation(_))));

    let mut suspicious = vec![0xff, 0x00];
    suspicious.extend_from_slice(b"mc:MustUnderstand='f'");
    assert!(matches!(wordcraft_docx::read(&replace_xml(&frames(true)[0].0, &suspicious)), Err(wordcraft_docx::DocxError::Xml(_))));
}

/// Malformed or unsupported XML qualification retains fallback images and the original payload.
#[test]
fn native_ink_xml_qualification_failures_keep_fallbacks_and_source_bytes() {
    let variants = [
        INK_XML.replace("xml:id=", "id="),
        INK_XML.replace("<trace ", "<trace bad:type=\"penUp\" "),
        INK_XML.replace("</ink>", "</ink>not XML whitespace"),
        INK_XML.replace("<definitions>", "<definitions><annotationXML>").replace("</definitions>", "</annotationXML></definitions>"),
        format!("<!DOCTYPE ink SYSTEM \"https://example.invalid/ink.dtd\">{INK_XML}"),
        INK_XML.replace("<trace ", "<trace xml:base=\"https://example.invalid/other.xml\" "),
        INK_XML.replace("<trace ", "<trace xmlns:a=\"urn:same\" xmlns:b=\"urn:same\" a:x=\"1\" b:x=\"2\" "),
    ];
    for ink in variants {
        for (bytes, grouped) in frames(true) {
            let mut doc = wordcraft_docx::read(&replace_xml(&bytes, ink.as_bytes())).unwrap();
            for _ in 0..2 {
                assert!(matches!(ink_graphic(&doc, grouped).items.as_slice(), [GraphicItem::Image { .. }]));
                let saved = wordcraft_docx::write(&doc).unwrap();
                let files = unzip(&saved);
                assert!(files.values().any(|bytes| bytes == ink.as_bytes()));
                assert!(files.values().any(|bytes| bytes == b"original preview bytes"));
                doc = wordcraft_docx::read(&saved).unwrap();
            }
        }
    }
}
