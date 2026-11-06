//! End-to-end checks that imported InkML is visible, hideable, and safely editable/exportable.

use std::io::{Cursor, Write};

use wordcraft_doc::props::Rgb;
use wordcraft_layout::display::{DisplayOptions, Draw, page_display};

use super::{Session, json, run};

mod completion;
mod defaults;
mod discrete;
mod inverse;
mod review;
mod xml;

/// Minimal InkML package part used in the native screen/PDF integration fixtures.
const INK: &str = r##"<ink xmlns="http://www.w3.org/2003/InkML"><brush><brushProperty name="color" value="#2563EB"/><brushProperty name="width" value="4" units="pt"/></brush><trace>0 0, 50 0, 50 25</trace></ink>"##;
/// Expected brush colour used to identify the fixture's stroke in display lists and pixels.
const BLUE: Rgb = Rgb(0x25, 0x63, 0xEB);

/// Create a tiny standards-shaped DOCX with native ink in the requested placement.
fn document(placement: &str) -> Vec<u8> {
    document_with_ink(placement, INK)
}

/// Package arbitrary synthetic InkML for the same placement, without importing any Word file.
fn document_with_ink(placement: &str, ink: &str) -> Vec<u8> {
    document_with_ink_bytes(placement, ink.as_bytes())
}

fn document_with_ink_bytes(placement: &str, ink: &[u8]) -> Vec<u8> {
    let inner = placement.ends_with("-inner");
    let placement = placement.strip_suffix("-inner").unwrap_or(placement);
    let content = if matches!(placement, "group" | "canvas") {
        r#"<w14:contentPart r:id="ink"><w14:xfrm><a:off x="254000" y="254000"/><a:ext cx="1371600" cy="736600"/></w14:xfrm></w14:contentPart>"#
    } else if inner {
        r#"<w14:contentPart r:id="ink"><w14:xfrm rot="5400000"><a:off x="127000" y="254000"/><a:ext cx="1371600" cy="736600"/></w14:xfrm></w14:contentPart>"#
    } else {
        r#"<w14:contentPart r:id="ink"><w14:xfrm><a:off x="0" y="0"/><a:ext cx="2743200" cy="1473200"/></w14:xfrm></w14:contentPart>"#
    };
    let payload = match placement {
        "group" => format!(
            r#"<a:graphicData uri="http://schemas.microsoft.com/office/word/2010/wordprocessingGroup"><wpg:wgp><wpg:cNvGrpSpPr/><wpg:grpSpPr><a:xfrm><a:off x="0" y="0"/><a:ext cx="2743200" cy="1473200"/><a:chOff x="0" y="0"/><a:chExt cx="2743200" cy="1473200"/></a:xfrm></wpg:grpSpPr>{content}</wpg:wgp></a:graphicData>"#
        ),
        "canvas" => format!(
            r#"<a:graphicData uri="http://schemas.microsoft.com/office/word/2010/wordprocessingCanvas"><wpc:wpc>{content}</wpc:wpc></a:graphicData>"#
        ),
        _ => format!(r#"<a:graphicData uri="http://schemas.microsoft.com/office/word/2010/wordprocessingInk">{content}</a:graphicData>"#),
    };
    let drawing = if placement == "direct" {
        r#"<w:contentPart r:id="ink"/>"#.to_string()
    } else if placement == "floating" {
        format!(
            r#"<w:drawing><wp:anchor><wp:positionH relativeFrom="page"><wp:posOffset>914400</wp:posOffset></wp:positionH><wp:positionV relativeFrom="page"><wp:posOffset>1828800</wp:posOffset></wp:positionV><wp:extent cx="2743200" cy="1473200"/><wp:wrapNone/><wp:docPr id="1" name="Original study strokes"/><a:graphic>{payload}</a:graphic></wp:anchor></w:drawing>"#
        )
    } else {
        format!(
            r#"<w:drawing><wp:inline><wp:extent cx="2743200" cy="1473200"/><wp:docPr id="1" name="Original study strokes"/><a:graphic>{payload}</a:graphic></wp:inline></w:drawing>"#
        )
    };
    let body = format!(
        r#"<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships" xmlns:wp="http://schemas.openxmlformats.org/drawingml/2006/wordprocessingDrawing" xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" xmlns:w14="http://schemas.microsoft.com/office/word/2010/wordml" xmlns:wpg="http://schemas.microsoft.com/office/word/2010/wordprocessingGroup" xmlns:wpc="http://schemas.microsoft.com/office/word/2010/wordprocessingCanvas"><w:body><w:p><w:r><w:t>Original text</w:t></w:r></w:p><w:p><w:r>{drawing}</w:r></w:p></w:body></w:document>"#
    );
    let mut zip = zip::ZipWriter::new(Cursor::new(Vec::new()));
    for (path, data) in [
        (
            "[Content_Types].xml",
            r#"<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Override PartName="/word/ink/ink1.xml" ContentType="application/inkml+xml"/></Types>"#,
        ),
        ("word/document.xml", body.as_str()),
        (
            "word/_rels/document.xml.rels",
            r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="ink" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/customXml" Target="ink/ink1.xml"/></Relationships>"#,
        ),
    ] {
        zip.start_file(path, zip::write::SimpleFileOptions::default()).unwrap();
        zip.write_all(data.as_bytes()).unwrap();
    }
    zip.start_file("word/ink/ink1.xml", zip::write::SimpleFileOptions::default()).unwrap();
    zip.write_all(ink).unwrap();
    zip.finish().unwrap().into_inner()
}

/// Synthetic transparent, rectangular pen with mapped coordinates and continuation fragments.
const ADVANCED_INK: &str = r##"<ink xmlns="http://www.w3.org/2003/InkML"><brush><brushProperty name="color" value="#2563EB"/><brushProperty name="width" value="8" units="pt"/><brushProperty name="height" value="4" units="pt"/><brushProperty name="tip" value="rectangle"/><brushProperty name="transparency" value="128"/></brush><context><canvasTransform><mapping type="affine"><affine>1.5 0 0,0 1.5 0</affine></mapping></canvasTransform></context><trace xml:id="first" continuation="begin">0 0,20 0</trace><trace continuation="end" priorRef="#first">20 10,0 10</trace></ink>"##;

/// Count the fixture's blue native-ink strokes through nested figure, clip, and rotation draws.
fn strokes(draws: &[Draw]) -> usize {
    draws
        .iter()
        .map(|draw| match draw {
            Draw::Ink { color, pts, .. } if *color == BLUE => {
                assert_eq!(pts.len(), 3);
                1
            }
            Draw::Figure { draws, .. } => strokes(draws),
            Draw::Clip { items, .. } | Draw::Rotated { items, .. } => strokes(items),
            _ => 0,
        })
        .sum()
}

/// Count pixels whose rendered colour is the fixture's distinctive blue pen.
fn blue_pixels(image: &wordcraft_render::Rendered) -> usize {
    image.pixels.as_chunks::<4>().0.iter().filter(|p| p[2] > 170 && p[0] < 100 && p[1] < 150).count()
}

/// Verify screen, Hide Ink, PDF vector, cap, and join behavior for every supported placement.
#[test]
fn native_ink_opens_renders_hides_and_exports_as_pdf_vectors() {
    for placement in ["inline", "floating", "group", "canvas", "direct"] {
        let doc = crate::io::open_bytes("ink.docx", &document(placement)).unwrap();
        let mut session = Session::new(doc);
        let layout = session.layout();
        let page = &layout.pages[0];
        assert_eq!(strokes(&page_display(&session.doc, page, &Default::default())), 1, "{placement}");
        let hidden = DisplayOptions { hide_ink: true, ..Default::default() };
        assert_eq!(strokes(&page_display(&session.doc, page, &hidden)), 0, "{placement}");
        let raster = wordcraft_render::render_page(&session.doc, page, 1.0, &Default::default());
        assert!(blue_pixels(&raster) > 100, "{placement}: strokes must actually paint");
        let raster =
            wordcraft_render::render_page(&session.doc, page, 1.0, &wordcraft_render::RenderOptions { display: hidden, ..Default::default() });
        assert_eq!(blue_pixels(&raster), 0, "{placement}: Hide Ink must also reach the rasterizer");
        let pdf = wordcraft_pdf::export_layout(&session.doc, &layout, &wordcraft_pdf::PdfOptions { compress: false, ..Default::default() }).unwrap();
        let raw = String::from_utf8_lossy(&pdf);
        let tokens: Vec<_> = raw.split_ascii_whitespace().collect();
        assert!(tokens.windows(2).any(|p| p == ["1", "J"]), "{placement}: PDF must have round stroke caps");
        assert!(tokens.windows(2).any(|p| p == ["1", "j"]), "{placement}: PDF must have round joins, matching the screen");
        assert!(!raw.contains("/Subtype /Image"), "{placement}: ink must remain vector, not a raster fallback");
    }
}

#[test]
fn unreadable_direct_native_ink_has_a_visible_sized_placeholder() {
    let doc = crate::io::open_bytes("ink.docx", &document_with_ink("direct", "malformed")).unwrap();
    let mut session = Session::new(doc);
    fn placeholder(draws: &[Draw]) -> Option<(f32, f32, f32, f32)> {
        draws.iter().find_map(|draw| match draw {
            Draw::Image { rect, media, .. } if media.is_empty() => Some((rect.x, rect.y, rect.w, rect.h)),
            Draw::Figure { draws, .. } => placeholder(draws),
            Draw::Clip { items, .. } | Draw::Rotated { items, .. } => placeholder(items),
            _ => None,
        })
    }
    for _ in 0..2 {
        let layout = session.layout();
        let page = &layout.pages[0];
        let draws = page_display(&session.doc, page, &Default::default());
        let (x, y, w, h) = placeholder(&draws).unwrap_or_else(|| panic!("direct ink placeholder draw: {draws:?}"));
        assert!(w >= 72.0 && h >= 36.0);
        let image = wordcraft_render::render_page(&session.doc, page, 1.0, &Default::default());
        let pixel = image.pixel((x + w / 2.0) as u32, (y + h / 2.0) as u32);
        assert_eq!(pixel, [0xD0, 0xD0, 0xD0, 255]);
        let saved = crate::io::save_bytes("ink.docx", &session.doc).unwrap();
        session = Session::new(crate::io::open_bytes("ink.docx", &saved).unwrap());
        assert!(session.doc.passthrough.values().any(|bytes| bytes.as_slice() == b"malformed"));
    }
}

/// Count the advanced fixture's compound outlines and check that layout keeps pen opacity.
fn outlines(draws: &[Draw]) -> usize {
    draws
        .iter()
        .map(|draw| match draw {
            Draw::Path { segs, fill: Some(wordcraft_layout::display::Paint::Solid(color, alpha)), stroke: None, .. } if *color == BLUE => {
                assert!((*alpha - 127.0 / 255.0).abs() < 1e-6);
                assert!(segs.len() > 10);
                1
            }
            Draw::Figure { draws, .. } => outlines(draws),
            Draw::Clip { items, .. } | Draw::Rotated { items, .. } => outlines(items),
            _ => 0,
        })
        .sum()
}

/// Screen and PDF share the same translucent compound geometry in every DOCX placement.
#[test]
fn advanced_native_ink_renders_hides_exports_and_survives_editing() {
    for placement in ["inline", "floating", "group", "canvas", "direct"] {
        let doc = crate::io::open_bytes("advanced.docx", &document_with_ink(placement, ADVANCED_INK)).unwrap();
        let mut session = Session::new(doc);
        let before = session.doc.body.clone();
        run(&mut session, "text.insert", json!({"text":"Edited "}));
        run(&mut session, "edit.undo", json!({}));
        assert_eq!(session.doc.body, before);
        run(&mut session, "edit.redo", json!({}));
        for _ in 0..2 {
            let saved = crate::io::save_bytes("advanced.docx", &session.doc).unwrap();
            session = Session::new(crate::io::open_bytes("advanced.docx", &saved).unwrap());
            assert!(session.doc.passthrough.values().any(|bytes| bytes.as_slice() == ADVANCED_INK.as_bytes()));
        }
        let layout = session.layout();
        let page = &layout.pages[0];
        assert_eq!(outlines(&page_display(&session.doc, page, &Default::default())), 1, "{placement}");
        let hidden = DisplayOptions { hide_ink: true, ..Default::default() };
        assert_eq!(outlines(&page_display(&session.doc, page, &hidden)), 0);
        let colored = |image: &wordcraft_render::Rendered| {
            image.pixels.as_chunks::<4>().0.iter().filter(|p| p[2] > 200 && p[0] < 220 && p[1] < 220 && p[2].saturating_sub(p[0]) > 25).count()
        };
        let raster = wordcraft_render::render_page(&session.doc, page, 1.0, &Default::default());
        assert!(colored(&raster) > 50, "{placement}: translucent nib must paint");
        let raster =
            wordcraft_render::render_page(&session.doc, page, 1.0, &wordcraft_render::RenderOptions { display: hidden, ..Default::default() });
        assert_eq!(colored(&raster), 0, "{placement}: Hide Ink must reach filled outlines");
        let pdf = wordcraft_pdf::export_layout(&session.doc, &layout, &wordcraft_pdf::PdfOptions { compress: false, ..Default::default() }).unwrap();
        let raw = String::from_utf8_lossy(&pdf);
        assert!(raw.contains("/ca "), "{placement}: PDF retains nonstroking opacity");
        assert!(!raw.contains("/Subtype /Image"), "{placement}: advanced ink remains vector");
    }
}

/// Find a native outline's bounds through figure wrappers for a center-pixel opacity assertion.
fn outline_bounds(draws: &[Draw]) -> Option<[f32; 4]> {
    use wordcraft_doc::graphic::PathSeg;
    draws.iter().find_map(|draw| match draw {
        Draw::Path { segs, fill: Some(wordcraft_layout::display::Paint::Solid(Rgb(255, 0, 0), _)), .. } => {
            let mut bounds = [f32::INFINITY, f32::INFINITY, f32::NEG_INFINITY, f32::NEG_INFINITY];
            for seg in segs {
                if let PathSeg::Move(x, y) | PathSeg::Line(x, y) = seg {
                    bounds[0] = bounds[0].min(*x);
                    bounds[1] = bounds[1].min(*y);
                    bounds[2] = bounds[2].max(*x);
                    bounds[3] = bounds[3].max(*y);
                }
            }
            Some(bounds)
        }
        Draw::Figure { draws, .. } => outline_bounds(draws),
        Draw::Clip { items, .. } | Draw::Rotated { items, .. } => outline_bounds(items),
        _ => None,
    })
}

/// A retraced translucent pen does not accumulate opacity within one trace, but two traces do.
#[test]
fn native_ink_outline_composites_once_per_stroke_not_per_polygon() {
    for count in [1, 2] {
        let traces = "<trace>0 0,50 0,0 0</trace>".repeat(count);
        let ink = format!(
            r##"<ink xmlns="http://www.w3.org/2003/InkML"><brush><brushProperty name="color" value="#FF0000"/><brushProperty name="width" value="4"/><brushProperty name="tip" value="rectangle"/><brushProperty name="transparency" value="0.5" units="%"/></brush>{traces}</ink>"##
        );
        let doc = crate::io::open_bytes("opacity.docx", &document_with_ink("inline", &ink)).unwrap();
        let mut session = Session::new(doc);
        let layout = session.layout();
        let page = &layout.pages[0];
        let [l, t, r, b] = outline_bounds(&page_display(&session.doc, page, &Default::default())).unwrap();
        let image = wordcraft_render::render_page(&session.doc, page, 1.0, &Default::default());
        let pixel = image.pixel(((l + r) / 2.0) as u32, ((t + b) / 2.0) as u32);
        let expected = if count == 1 { 128 } else { 64 };
        assert!(pixel[0] > 250 && pixel[1].abs_diff(expected) <= 2 && pixel[2].abs_diff(expected) <= 2, "{count} strokes: {pixel:?}");
    }
}

/// Editing history and successive DOCX saves must preserve ink data and its visible group stroke.
#[test]
fn native_ink_survives_edit_undo_redo_and_repeated_docx_saves() {
    let doc = crate::io::open_bytes("ink.docx", &document("group")).unwrap();
    let mut session = Session::new(doc);
    let before = session.doc.body.clone();
    run(&mut session, "text.insert", json!({"text": "Edited "}));
    run(&mut session, "edit.undo", json!({}));
    assert_eq!(session.doc.body, before);
    run(&mut session, "edit.redo", json!({}));
    assert!(session.doc.plain_text(wordcraft_doc::StoryRef::Body).starts_with("Edited Original"));
    for _ in 0..2 {
        let saved = crate::io::save_bytes("ink.docx", &session.doc).unwrap();
        let doc = crate::io::open_bytes("ink.docx", &saved).unwrap();
        assert!(doc.passthrough.values().any(|bytes| bytes.as_slice() == INK.as_bytes()));
        session = Session::new(doc);
        let layout = session.layout();
        assert_eq!(strokes(&page_display(&session.doc, &layout.pages[0], &Default::default())), 1);
    }
}
