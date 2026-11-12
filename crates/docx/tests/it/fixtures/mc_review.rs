//! Focused regressions for retained XML and MCE review findings on #6035.
use super::*;

/// ECMA-376 Part 3 §9.4(3)(b): no selected branch removes only the AlternateContent.
#[test]
fn unsupported_choice_without_fallback_contributes_no_content() {
    for namespace in ["urn:future", "http://schemas.microsoft.com/office/drawing/2010/main"] {
        let alternate = format!(
            r#"<mc:AlternateContent xmlns:f="{namespace}"><mc:Choice Requires="f" mc:MustUnderstand="f"><w:r><w:t>unselected</w:t></w:r></mc:Choice></mc:AlternateContent>"#
        );
        for body in [
            format!(r#"<w:p><w:r><w:t>before</w:t></w:r>{alternate}<w:r><w:t>after</w:t></w:r></w:p>"#),
            format!(r#"<w:p><w:r><w:t>before</w:t></w:r></w:p>{alternate}<w:p><w:r><w:t>after</w:t></w:r></w:p>"#),
        ] {
            let mut doc = wordcraft_docx::read(&docx(&body, &[], &[])).unwrap();
            for _ in 0..2 {
                let plain = doc.plain_text(wordcraft_doc::StoryRef::Body);
                assert!(plain.contains("before") && plain.contains("after") && !plain.contains("unselected"), "{plain}");
                doc = wordcraft_docx::read(&wordcraft_docx::write(&doc).unwrap()).unwrap();
            }
        }
    }
    // An empty wrapper is not the valid, unsupported-Choice case (Part 3 §7.5).
    assert!(matches!(wordcraft_docx::read(&docx("<mc:AlternateContent/>", &[], &[])), Err(wordcraft_docx::DocxError::Xml(_))));
    // The wrapper remains active, unlike its unselected Choice.
    let active = r#"<mc:AlternateContent xmlns:f="urn:future" mc:MustUnderstand="f"><mc:Choice Requires="f"/></mc:AlternateContent>"#;
    assert!(matches!(wordcraft_docx::read(&docx(active, &[], &[])), Err(wordcraft_docx::DocxError::Xml(_))));
}

/// Resolve QName *values* as XML Schema QNames, including inherited default namespaces.
fn expanded_values(xml: &str, attr: &str) -> Vec<(String, String)> {
    use quick_xml::events::Event;
    use quick_xml::name::{QName, ResolveResult};
    let mut reader = quick_xml::reader::NsReader::from_str(xml);
    let mut found = Vec::new();
    loop {
        match reader.read_event().unwrap() {
            Event::Start(e) | Event::Empty(e) => {
                for a in e.attributes().map(Result::unwrap).filter(|a| a.key.local_name().as_ref() == attr.as_bytes()) {
                    let value = a.normalized_value(quick_xml::XmlVersion::Explicit1_0).unwrap();
                    let (namespace, local) = reader.resolver().resolve_element(QName(value.as_bytes()));
                    let namespace = match namespace {
                        ResolveResult::Bound(ns) => String::from_utf8(ns.as_ref().to_vec()).unwrap(),
                        ResolveResult::Unbound => String::new(),
                        ResolveResult::Unknown(prefix) => panic!("unbound QName prefix {} in {xml}", String::from_utf8_lossy(&prefix)),
                    };
                    found.push((namespace, String::from_utf8(local.as_ref().to_vec()).unwrap()));
                }
            }
            Event::Eof => return found,
            _ => {}
        }
    }
}

/// A retained graphic must preserve QName values, not merely remain parseable XML.
#[test]
fn retained_drawing_qnames_keep_ancestor_and_rebound_namespace_bindings() {
    for attr in ["xsi:type", "f:kind", "ordinary"] {
        for default in [false, true] {
            let mut files = unzip(&ink_docx(false, false));
            let outer = if default { "Outer" } else { "q:Outer" };
            let inner = if default { "Inner" } else { "q:Inner" };
            let metadata = format!(r#"<ink:annotation {attr}="{outer}"/><ink:annotation xmlns:q="urn:inner" xmlns="urn:inner" {attr}="{inner}"/>"#);
            let xml = text(&files, "word/document.xml")
                .replace("<w:drawing>", r#"<w:drawing xmlns:q="urn:outer" xmlns="urn:outer" xmlns:f="urn:metadata" xmlns:ink="http://www.w3.org/2003/InkML" xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance">"#)
                .replace("</a:graphicData>", &format!("{metadata}</a:graphicData>"));
            let local = attr.rsplit(':').next().unwrap();
            let expected = vec![("urn:outer".into(), "Outer".into()), ("urn:inner".into(), "Inner".into())];
            assert_eq!(expanded_values(&xml, local), expected);
            files.insert("word/document.xml".into(), xml.into_bytes());
            let entries: Vec<_> = files.iter().map(|(p, b)| (p.as_str(), b.as_slice())).collect();
            let mut doc = wordcraft_docx::read(&zip(&entries)).unwrap();
            assert_eq!(graphics(&doc).len(), 1);
            for _ in 0..2 {
                let saved = wordcraft_docx::write(&doc).unwrap();
                let xml = text(&unzip(&saved), "word/document.xml");
                doc = wordcraft_docx::read(&saved).unwrap();
                assert_eq!(graphics(&doc).len(), 1);
                assert_eq!(expanded_values(&xml, local), expected);
            }
            doc.core.title = "unrelated edit".into();
            assert!(matches!(wordcraft_docx::write(&doc), Err(wordcraft_docx::DocxError::Preservation(_))));
        }
    }
}

/// Opaque text may also be a QName, including one using a default namespace.
#[test]
fn retained_drawing_opaque_text_preserves_original_namespace_scope() {
    for value in ["q:Outer", "Outer"] {
        let mut files = unzip(&ink_docx(false, false));
        let xml = text(&files, "word/document.xml")
            .replace("<w:drawing>", r#"<w:drawing xmlns:q="urn:outer" xmlns="urn:outer" xmlns:ink="http://www.w3.org/2003/InkML">"#)
            .replace("</a:graphicData>", &format!("<ink:annotation>{value}</ink:annotation></a:graphicData>"));
        files.insert("word/document.xml".into(), xml.into_bytes());
        let entries: Vec<_> = files.iter().map(|(p, b)| (p.as_str(), b.as_slice())).collect();
        let mut doc = wordcraft_docx::read(&zip(&entries)).unwrap();
        for _ in 0..2 {
            let saved = wordcraft_docx::write(&doc).unwrap();
            assert_eq!(unzip(&saved), files, "opaque QName text lost original scope: {value}");
            doc = wordcraft_docx::read(&saved).unwrap();
        }
    }
}

/// Colon restrictions here are a safe-replay policy, not a claim about OPC IRI validity.
#[test]
fn required_source_rejects_colons_in_every_zip_name_segment() {
    let body = r#"<w:p xmlns:f="urn:future" mc:Ignorable="f" mc:PreserveElements="f:opaque"><f:opaque/><w:r><w:t>visible</w:t></w:r></w:p>"#;
    for name in ["C:/outside.xml", "word/media/image.bin:payload", "word/media:payload/image.bin"] {
        let source = docx(body, &[], &[(name, "opaque payload")]);
        let result = wordcraft_docx::read(&source);
        assert!(matches!(result, Err(wordcraft_docx::DocxError::Preservation(_))), "unsafe retained name accepted: {name}");
    }
    let safe = docx(body, &[], &[("word/media/image.bin", "opaque payload")]);
    let doc = wordcraft_docx::read(&safe).unwrap();
    assert_eq!(unzip(&wordcraft_docx::write(&doc).unwrap()), unzip(&safe));
}
