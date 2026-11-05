use std::io::{Cursor, Write};

use wordcraft_doc::props::Rgb;
use wordcraft_layout::display::{DisplayOptions, Draw, page_display};

use super::{Session, json, run};

const INK: &str = r##"<ink xmlns="http://www.w3.org/2003/InkML"><brush><brushProperty name="color" value="#2563EB"/><brushProperty name="width" value="4" units="pt"/></brush><trace>0 0, 50 0, 50 25</trace></ink>"##;
const BLUE: Rgb = Rgb(0x25, 0x63, 0xEB);

fn document(placement: &str) -> Vec<u8> {
    let content = if matches!(placement, "group" | "canvas") {
        r#"<w14:contentPart r:id="ink"><w14:xfrm><a:off x="254000" y="254000"/><a:ext cx="1371600" cy="736600"/></w14:xfrm></w14:contentPart>"#
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
        ("word/ink/ink1.xml", INK),
    ] {
        zip.start_file(path, zip::write::SimpleFileOptions::default()).unwrap();
        zip.write_all(data.as_bytes()).unwrap();
    }
    zip.finish().unwrap().into_inner()
}

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

fn blue_pixels(image: &wordcraft_render::Rendered) -> usize {
    image.pixels.as_chunks::<4>().0.iter().filter(|p| p[2] > 170 && p[0] < 100 && p[1] < 150).count()
}

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
