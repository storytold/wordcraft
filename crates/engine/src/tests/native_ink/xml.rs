use super::super::{json, run};
use super::{DisplayOptions, INK, Session, blue_pixels, document_with_ink, document_with_ink_bytes};

#[test]
fn native_ink_xml_versions_encodings_and_namespace_aliases_match_screen_and_pdf_controls() {
    let mut alias = INK.replace("xmlns=", "xmlns:筆=").replace("/InkML", "/Ink&#77;L");
    for tag in ["ink", "brush", "brushProperty", "trace"] {
        alias = alias.replace(&format!("<{tag}"), &format!("<筆:{tag}")).replace(&format!("</{tag}>"), &format!("</筆:{tag}>"));
    }
    let xml11 = format!(
        "<?xml version=\"1.1\" encoding=\"UTF-8\"?>{}",
        INK.replace("<trace>", "<trace\u{85}type=\"penDown\">")
            .replace("</brush>", "</brush><annotationXML><meta xmlns=\"urn:metadata\">&#x1;\u{2028}</meta></annotationXML>")
    );
    let mut variants = vec![alias.into_bytes(), xml11.as_bytes().to_vec(), [vec![0xef, 0xbb, 0xbf], xml11.as_bytes().to_vec()].concat()];
    for little in [false, true] {
        let mut bytes = if little { vec![0xff, 0xfe] } else { vec![0xfe, 0xff] };
        for unit in xml11.replace("UTF-8", "UTF-16").encode_utf16() {
            bytes.extend(if little { unit.to_le_bytes() } else { unit.to_be_bytes() });
        }
        variants.push(bytes);
    }
    for ink in variants {
        for placement in ["inline", "floating", "group", "canvas", "direct"] {
            let doc = crate::io::open_bytes("control.docx", &document_with_ink(placement, INK)).unwrap();
            let mut reference = Session::new(doc);
            let layout = reference.layout();
            let expected = wordcraft_render::render_page(&reference.doc, &layout.pages[0], 1.0, &Default::default());
            let doc = crate::io::open_bytes("xml.docx", &document_with_ink_bytes(placement, &ink)).unwrap();
            let mut session = Session::new(serde_json::from_slice(&serde_json::to_vec(&doc).unwrap()).unwrap());
            let before = session.doc.body.clone();
            run(&mut session, "text.insert", json!({"text":"Edit "}));
            run(&mut session, "edit.undo", json!({}));
            assert_eq!(session.doc.body, before);
            for _ in 0..3 {
                let layout = session.layout();
                let page = &layout.pages[0];
                let actual = wordcraft_render::render_page(&session.doc, page, 1.0, &Default::default());
                assert!(blue_pixels(&actual) > 100);
                assert_eq!(actual.pixels, expected.pixels, "{placement}: XML representation must not change ink");
                let hidden = wordcraft_render::render_page(
                    &session.doc,
                    page,
                    1.0,
                    &wordcraft_render::RenderOptions { display: DisplayOptions { hide_ink: true, ..Default::default() }, ..Default::default() },
                );
                assert_eq!(blue_pixels(&hidden), 0);
                let pdf = wordcraft_pdf::export_layout(&session.doc, &layout, &Default::default()).unwrap();
                assert!(!String::from_utf8_lossy(&pdf).contains("/Subtype /Image"));
                assert!(session.doc.passthrough.values().any(|bytes| bytes.as_slice() == ink.as_slice()));
                let saved = crate::io::save_bytes("xml.docx", &session.doc).unwrap();
                session = Session::new(crate::io::open_bytes("xml.docx", &saved).unwrap());
            }
        }
    }
}
