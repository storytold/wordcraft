//! Integration regressions for native ink decoding, fallbacks, package preservation, and budgets.

use super::*;
use wordcraft_doc::graphic::{Graphic, GraphicItem, PathSeg};

/// Replace the fixture package's InkML part without changing its DOCX drawing or relationships.
fn replace_ink(bytes: &[u8], ink: &str) -> Vec<u8> {
    let mut files = unzip(bytes);
    files.insert("word/ink/ink1.xml".into(), ink.as_bytes().to_vec());
    let entries: Vec<_> = files.iter().map(|(path, bytes)| (path.as_str(), bytes.as_slice())).collect();
    zip(&entries)
}

/// Get the ink graphic from a fixture, selecting its group/canvas member when requested.
fn ink_graphic(d: &Document, grouped: bool) -> std::sync::Arc<Graphic> {
    let obj = if grouped { group_children(d)[1].obj.clone() } else { graphics(d)[0].clone() };
    let InlineObject::Graphic { graphic, .. } = obj else { panic!("expected ink") };
    graphic
}

/// Build drawing-level, group, and canvas fixtures with or without a bitmap fallback.
fn frames(fallback: bool) -> Vec<(Vec<u8>, bool)> {
    vec![
        (ink_docx(fallback, true), false),
        (nested_ink_docx(false, &ink_member(914400, fallback), fallback), true),
        (nested_ink_docx(true, &ink_member(914400, fallback), fallback), true),
    ]
}

/// Check vector decoding and original InkML preservation across placement and format round trips.
#[test]
fn native_ink_strokes_render_in_all_frames_and_survive_json_and_two_docx_saves() {
    for fallback in [false, true] {
        for (bytes, grouped) in frames(fallback) {
            let mut doc = wordcraft_docx::read(&bytes).unwrap();
            let original = ink_graphic(&doc, grouped);
            let [GraphicItem::Path { segs, stroke, stroke_width, .. }] = original.items.as_slice() else { panic!("{:?}", original.items) };
            assert_eq!(*stroke, Some(wordcraft_doc::props::Rgb(0x25, 0x63, 0xEB)));
            let fitted_width = 0.08 * (original.w / 2.08 + original.h / 1.08) / 2.0;
            assert!((*stroke_width - fitted_width).abs() < 0.001);
            assert!(matches!(segs.as_slice(), [PathSeg::Move(..), PathSeg::Line(..), PathSeg::Line(..)]));
            doc = serde_json::from_slice(&serde_json::to_vec(&doc).unwrap()).unwrap();
            for _ in 0..2 {
                let saved = wordcraft_docx::write(&doc).unwrap();
                check_ink(&unzip(&saved));
                doc = wordcraft_docx::read(&saved).unwrap();
                assert_eq!(ink_graphic(&doc, grouped).items, original.items);
            }
        }
    }
}

/// Unsupported visual ink semantics use their picture while preserving bytes across repeated saves.
#[test]
fn native_ink_unsupported_features_keep_the_preview_and_original_bytes() {
    let variants = [
        INK_XML.replace("0 0, 1 1, 2 0", "0 0, NaN 1"),
        INK_XML.replace("</brush>", r#"<brushProperty name="transparency" value="128"/></brush>"#),
        INK_XML.replace("</context>", "<canvasTransform/></context>"),
        "<not-ink>broken".to_string(),
    ];
    for ink in variants {
        for (bytes, grouped) in frames(true) {
            let mut doc = wordcraft_docx::read(&replace_ink(&bytes, &ink)).unwrap();
            for _ in 0..2 {
                let graphic = ink_graphic(&doc, grouped);
                assert!(matches!(graphic.items.as_slice(), [GraphicItem::Image { media, .. }] if doc.media.contains_key(media)));
                let saved = wordcraft_docx::write(&doc).unwrap();
                let files = unzip(&saved);
                assert!(files.values().any(|bytes| bytes == ink.as_bytes()));
                assert!(files.values().any(|bytes| bytes == b"original preview bytes"));
                doc = wordcraft_docx::read(&saved).unwrap();
            }
        }
    }
}

/// An unsupported ink part without a picture fallback retains the visible placeholder.
#[test]
fn native_ink_without_a_preview_keeps_a_placeholder_when_it_cannot_be_drawn() {
    for (bytes, grouped) in frames(false) {
        let doc = wordcraft_docx::read(&replace_ink(&bytes, "malformed")).unwrap();
        assert!(matches!(ink_graphic(&doc, grouped).items.as_slice(), [GraphicItem::Image { media, .. }] if media.is_empty()));
        let files = unzip(&wordcraft_docx::write(&doc).unwrap());
        assert!(files.values().any(|bytes| bytes == b"malformed"));
    }
}

/// A direct run-level ink part derives its graphic frame from the decoded stroke bounds.
#[test]
fn direct_native_ink_has_intrinsic_dimensions_and_visible_strokes() {
    let bytes = docx(
        r#"<w:p><w:r><w:contentPart r:id="rInk"/></w:r></w:p>"#,
        &[("rInk", "customXml", "ink/ink1.xml")],
        &[("[Content_Types].xml", INK_TYPES), ("word/ink/ink1.xml", INK_XML)],
    );
    let mut doc = wordcraft_docx::read(&bytes).unwrap();
    for _ in 0..3 {
        let objects = graphics(&doc);
        let InlineObject::Graphic { w, h, graphic, .. } = &objects[0] else { panic!() };
        assert!(*w > 50.0 && *h > 25.0);
        assert_eq!((*w, *h), (graphic.w, graphic.h));
        assert!(matches!(graphic.items.as_slice(), [GraphicItem::Path { .. }]));
        doc = wordcraft_docx::read(&wordcraft_docx::write(&doc).unwrap()).unwrap();
    }
}

/// Keep inner-offset or resized ink on its preview; continue drawing a supported rotated frame.
#[test]
fn native_ink_inner_placement_uses_the_preview_but_pure_rotation_keeps_vectors() {
    for (from, to, vectors) in [
        ("<q:xfrm>", r#"<q:xfrm rot="2700000">"#, true),
        (r#"<a:off x="0" y="0"/>"#, r#"<a:off x="127000" y="254000"/>"#, false),
        (r#"<a:ext cx="1828800" cy="914400"/>"#, r#"<a:ext cx="914400" cy="457200"/>"#, false),
    ] {
        let mut files = unzip(&ink_docx(true, false));
        let body = text(&files, "word/document.xml").replace(from, to);
        files.insert("word/document.xml".into(), body.into_bytes());
        let entries: Vec<_> = files.iter().map(|(path, bytes)| (path.as_str(), bytes.as_slice())).collect();
        let doc = wordcraft_docx::read(&zip(&entries)).unwrap();
        let graphic = ink_graphic(&doc, false);
        assert_eq!(matches!(graphic.items.first(), Some(GraphicItem::Path { .. })), vectors);
        if !vectors {
            assert!(matches!(graphic.items.as_slice(), [GraphicItem::Image { media, .. }] if doc.media.contains_key(media)));
        }
        check_ink(&unzip(&wordcraft_docx::write(&doc).unwrap()));
    }
}

/// Repeated references charge both first-time decoding and copied vector geometry to the work cap.
#[test]
fn native_ink_decoding_and_copying_share_the_document_graphic_budget() {
    let mut files = unzip(&ink_docx(false, false));
    let mut body = text(&files, "word/document.xml");
    let start = body.find("<w:drawing>").unwrap();
    let end = body.find("</w:drawing>").unwrap() + "</w:drawing>".len();
    let drawing = body[start..end].to_string();
    let copies = (0..64).map(|_| format!("<w:p><w:r>{drawing}</w:r></w:p>")).collect::<String>();
    body = body.replace("</w:body>", &format!("{copies}</w:body>"));
    files.insert("word/document.xml".into(), body.into_bytes());
    let ink = format!(r#"<ink xmlns="http://www.w3.org/2003/InkML"><trace>{}</trace></ink>"#, "0 0,1 1,".repeat(30_000));
    files.insert("word/ink/ink1.xml".into(), ink.as_bytes().to_vec());
    let entries: Vec<_> = files.iter().map(|(path, bytes)| (path.as_str(), bytes.as_slice())).collect();
    let doc = wordcraft_docx::read(&zip(&entries)).unwrap();
    let objects = graphics(&doc);
    assert_eq!(objects.len(), 65, "budget must not discard native objects");
    let mut segments = 0;
    let mut placeholders = 0;
    for object in objects {
        let InlineObject::Graphic { graphic, .. } = object else { panic!() };
        assert!(graphic.source.is_some());
        for item in &graphic.items {
            match item {
                GraphicItem::Path { segs, .. } => segments += segs.len(),
                GraphicItem::Image { .. } => placeholders += 1,
                _ => panic!("unexpected ink item"),
            }
        }
    }
    assert!((60_000..2_000_000).contains(&segments));
    assert!(placeholders > 0, "copied geometry also consumes the shared budget");
}
