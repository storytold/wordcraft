use super::*;

/// Repackage entries after a synthetic mutation.
fn repack(files: &std::collections::BTreeMap<String, Vec<u8>>) -> Vec<u8> {
    zip(&files.iter().map(|(path, bytes)| (path.as_str(), bytes.as_slice())).collect::<Vec<_>>())
}

/// InkML MIME identity is ASCII case-insensitive for both OPC overrides and defaults.
#[test]
fn mixed_case_ink_mime_preserves_vectors_and_original_parts() {
    let direct = docx(
        r#"<w:p><w:r><w:contentPart r:id="rInk"/></w:r></w:p>"#,
        &[("rInk", "customXml", "ink/ink1.xml")],
        &[("[Content_Types].xml", INK_TYPES), ("word/ink/ink1.xml", INK_XML)],
    );
    let mut placements = frames(true);
    let mut floating = unzip(&ink_docx(false, false));
    let body = text(&floating, "word/document.xml").replace("wp:inline", "wp:anchor");
    floating.insert("word/document.xml".into(), body.into_bytes());
    placements.push((repack(&floating), false));
    placements.push((ink_docx(false, false), false));
    placements.push((direct, false));
    for (bytes, grouped) in placements {
        let expected = ink_graphic(&wordcraft_docx::read(&bytes).unwrap(), grouped).items.clone();
        for default in [false, true] {
            let mut files = unzip(&bytes);
            let types = text(&files, "[Content_Types].xml").replace("application/inkml+xml", "Application/InkML+XML");
            let types = if default {
                let ink = files.remove("word/ink/ink1.xml").unwrap();
                files.insert("word/ink/ink1.inkml".into(), ink);
                let rels = text(&files, "word/_rels/document.xml.rels").replace("ink/ink1.xml", "ink/ink1.inkml");
                files.insert("word/_rels/document.xml.rels".into(), rels.into_bytes());
                types.replace(r#"Override PartName="/word/ink/ink1.xml""#, r#"Default Extension="InkML""#)
            } else {
                types
            };
            files.insert("[Content_Types].xml".into(), types.into_bytes());
            let mut doc = wordcraft_docx::read(&repack(&files)).unwrap();
            doc.core.title = "unrelated edit".into();
            for _ in 0..2 {
                assert_eq!(ink_graphic(&doc, grouped).items, expected, "default={default}, grouped={grouped}");
                let saved = wordcraft_docx::write(&doc).unwrap();
                let out = unzip(&saved);
                assert!(out.values().any(|bytes| bytes == INK_XML.as_bytes()));
                assert!(text(&out, "[Content_Types].xml").contains("Application/InkML+XML"));
                doc = wordcraft_docx::read(&saved).unwrap();
            }
        }
    }
}

/// Recognized mixed-case ink belongs to its drawing, not a second orphaned custom XML store.
#[test]
fn deleting_mixed_case_ink_does_not_leave_an_orphan_store() {
    let mut files = unzip(&ink_docx(false, false));
    let types = text(&files, "[Content_Types].xml").replace("application/inkml+xml", "Application/InkML+XML");
    files.insert("[Content_Types].xml".into(), types.into_bytes());
    let mut doc = wordcraft_docx::read(&repack(&files)).unwrap();
    assert_eq!(graphics(&doc).len(), 1);
    doc.body.clear();
    let saved = wordcraft_docx::write(&doc).unwrap();
    assert!(!unzip(&saved).values().any(|bytes| bytes == INK_XML.as_bytes()));
}

/// Build an unreferenced typed part, with no relationship or past the bibliography candidate cap.
fn hidden_ink(mime: &str, directive: &str, related: bool) -> Vec<u8> {
    let mut files = unzip(&frames(true)[0].0);
    let ink = format!(
        r#"<ink xmlns="http://www.w3.org/2003/InkML" xmlns:mc="http://schemas.openxmlformats.org/markup-compatibility/2006" xmlns:f="urn:unsupported" {directive}><f:opaque/></ink>"#
    );
    files.insert("word/ink/hidden.xml".into(), ink.into_bytes());
    let types = text(&files, "[Content_Types].xml")
        .replace("</Types>", &format!(r#"<Override PartName="/word/ink/hidden.xml" ContentType="{mime}"/></Types>"#));
    files.insert("[Content_Types].xml".into(), types.into_bytes());
    if related {
        let extra: String = (0..64)
            .map(|i| format!(r#"<Relationship Id="other{i}" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/customXml" Target="unused{i}.xml"/>"#))
            .collect();
        let hidden = r#"<Relationship Id="hidden" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/customXml" Target="ink/hidden.xml"/>"#;
        let rels = text(&files, "word/_rels/document.xml.rels").replace("</Relationships>", &format!("{extra}{hidden}</Relationships>"));
        files.insert("word/_rels/document.xml.rels".into(), rels.into_bytes());
    }
    repack(&files)
}

/// Package-wide MC inspection must not depend on the MIME spelling or relationship candidate cap.
#[test]
fn mixed_case_ink_mime_cannot_bypass_must_understand() {
    for mime in ["application/inkml+xml", "Application/InkML+XML"] {
        for related in [false, true] {
            let source = hidden_ink(mime, r#"mc:MustUnderstand="f""#, related);
            assert!(matches!(wordcraft_docx::read(&source), Err(wordcraft_docx::DocxError::Xml(_))), "{mime}, related={related}");
        }
    }
}

/// An unreferenced preservation directive requires exact source replay, never orphan cleanup.
#[test]
fn mixed_case_ink_mime_cannot_lose_preserve_elements() {
    for mime in ["application/inkml+xml", "Application/InkML+XML"] {
        for related in [false, true] {
            let source = hidden_ink(mime, r#"mc:Ignorable="f" mc:PreserveElements="f:*""#, related);
            let mut doc = wordcraft_docx::read(&source).unwrap();
            assert_eq!(unzip(&wordcraft_docx::write(&doc).unwrap()), unzip(&source), "{mime}, related={related}");
            doc.core.title = "changed".into();
            assert!(matches!(wordcraft_docx::write(&doc), Err(wordcraft_docx::DocxError::Preservation(_))), "{mime}, related={related}");
        }
    }
}

/// A normal bibliography candidate is checked, but a referenced part past its cap also must be.
#[test]
fn mixed_case_referenced_ink_keeps_compatibility_guards_past_candidate_limit() {
    for capped in [false, true] {
        for preserve in [false, true] {
            for (source, _) in frames(true) {
                let directive = if preserve { r#"mc:Ignorable="f" mc:PreserveElements="f:*""# } else { r#"mc:MustUnderstand="f""# };
                let ink = format!(
                    r#"<ink xmlns="http://www.w3.org/2003/InkML" xmlns:mc="http://schemas.openxmlformats.org/markup-compatibility/2006" xmlns:f="urn:unsupported" {directive}><f:opaque/></ink>"#
                );
                let mut files = unzip(&replace_ink(&source, &ink));
                let types = text(&files, "[Content_Types].xml").replace("application/inkml+xml", "Application/InkML+XML");
                files.insert("[Content_Types].xml".into(), types.into_bytes());
                if capped {
                    let extra: String = (0..64)
                        .map(|i| format!(r#"<Relationship Id="other{i}" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/customXml" Target="unused{i}.xml"/>"#))
                        .collect();
                    let rels = text(&files, "word/_rels/document.xml.rels")
                        .replace(r#"<Relationship Id="rInk""#, &format!(r#"{extra}<Relationship Id="rInk""#));
                    files.insert("word/_rels/document.xml.rels".into(), rels.into_bytes());
                }
                let source = repack(&files);
                if preserve {
                    let mut doc = wordcraft_docx::read(&source).unwrap();
                    assert_eq!(unzip(&wordcraft_docx::write(&doc).unwrap()), files, "capped={capped}");
                    doc.core.title = "changed".into();
                    assert!(matches!(wordcraft_docx::write(&doc), Err(wordcraft_docx::DocxError::Preservation(_))), "capped={capped}");
                } else {
                    assert!(matches!(wordcraft_docx::read(&source), Err(wordcraft_docx::DocxError::Xml(_))), "capped={capped}");
                }
            }
        }
    }
}

/// ENTITY/DOCTYPE examples in comments, CDATA and processing instructions are not declarations.
#[test]
fn entity_examples_in_ink_metadata_do_not_block_import_or_edits() {
    for metadata in [
        r#"<!-- Example: <!ENTITY foo "bar"> <!DOCTYPE ink SYSTEM "example.dtd"> -->"#,
        r#"<annotation><![CDATA[Example: <!ENTITY foo "bar"> <!DOCTYPE ink>]]></annotation>"#,
        r#"<?example <!ENTITY foo "bar"> <!DOCTYPE ink> ?>"#,
    ] {
        let ink = INK_XML.replace("<definitions>", &format!("{metadata}<definitions>"));
        for bytes in [ink.as_bytes().to_vec(), xml::utf16(&ink, true), xml::utf16(&ink, false)] {
            for (source, grouped) in frames(true) {
                let expected = ink_graphic(&wordcraft_docx::read(&source).unwrap(), grouped).items.clone();
                let mut doc = wordcraft_docx::read(&xml::replace_xml(&source, &bytes)).unwrap();
                assert_eq!(ink_graphic(&doc, grouped).items, expected);
                doc.core.title = "safe unrelated edit".into();
                let saved = wordcraft_docx::write(&doc).unwrap();
                assert!(unzip(&saved).values().any(|part| part == &bytes));
            }
        }
    }
}

/// A DOCTYPE example alone must not require source replay; a real DTD still must.
#[test]
fn doctype_examples_do_not_require_replay_but_real_dtds_do() {
    for prefix in [
        r#"<!-- Example: <!DOCTYPE ink SYSTEM 'example.dtd'> -->"#,
        r#"<!DOCTYPE ink [<!-- Example: <!ENTITY foo 'bar'> -->]>"#,
        r#"<!DOCTYPE ink SYSTEM 'https://example.invalid/<!ENTITY-example'>"#,
    ] {
        let ink = format!("{prefix}{INK_XML}");
        let source = replace_ink(&frames(true)[0].0, &ink);
        let mut doc = wordcraft_docx::read(&source).unwrap();
        doc.core.title = "changed".into();
        if prefix.starts_with("<!--") {
            let saved = wordcraft_docx::write(&doc).unwrap();
            assert!(unzip(&saved).values().any(|bytes| bytes == ink.as_bytes()));
        } else {
            assert!(matches!(wordcraft_docx::write(&doc), Err(wordcraft_docx::DocxError::Preservation(_))));
        }
    }
}

/// Real entity declarations stay fatal, including parameter/external entities and hidden MC.
#[test]
fn unsafe_ink_entities_remain_rejected() {
    for dtd in [
        r#"<!DOCTYPE ink [<!ENTITY foo "bar">]>"#,
        r#"<!DOCTYPE ink [<!ENTITY foo SYSTEM "file:///never-read">]>"#,
        r#"<!DOCTYPE ink [<!ENTITY % foo SYSTEM "https://example.invalid/never-fetch"> %foo;]>"#,
        r#"<!DOCTYPE ink [<!ENTITY foo "&#60;x xmlns:mc='http://schemas.openxmlformats.org/markup-compatibility/2006' mc:Must&#85;nderstand='f'/>">]>"#,
        r#"<!ENTITY foo "bar">"#,
    ] {
        let ink = format!("{dtd}{INK_XML}");
        for bytes in [ink.as_bytes().to_vec(), xml::utf16(&ink, true), xml::utf16(&ink, false)] {
            let source = xml::replace_xml(&frames(true)[0].0, &bytes);
            assert!(matches!(wordcraft_docx::read(&source), Err(wordcraft_docx::DocxError::Xml(_))), "{dtd}");
        }
    }
}
