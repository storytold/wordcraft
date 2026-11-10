//! Cross-feature regressions for native ink resource limits and export fallbacks.
use super::{ADVANCED_INK, INK, Session, document_with_ink, json, run};
use std::{
    collections::BTreeMap,
    io::{Cursor, Read, Write},
};
use wordcraft_doc::{Block, Document, InlineObject, graphic::GraphicKind};

/// Reads every named entry from a synthetic DOCX zip into a map.
fn unpack(bytes: &[u8]) -> BTreeMap<String, Vec<u8>> {
    let mut zip = zip::ZipArchive::new(Cursor::new(bytes)).unwrap();
    (0..zip.len())
        .map(|i| {
            let mut entry = zip.by_index(i).unwrap();
            let mut bytes = Vec::new();
            entry.read_to_end(&mut bytes).unwrap();
            (entry.name().to_string(), bytes)
        })
        .collect()
}

/// Writes the named entries back into a synthetic DOCX zip.
fn pack(files: BTreeMap<String, Vec<u8>>) -> Vec<u8> {
    let mut zip = zip::ZipWriter::new(Cursor::new(Vec::new()));
    for (name, bytes) in files {
        zip.start_file(name, zip::write::SimpleFileOptions::default()).unwrap();
        zip.write_all(&bytes).unwrap();
    }
    zip.finish().unwrap().into_inner()
}

/// Replaces a required string in one UTF-8 package part.
fn replace(files: &mut BTreeMap<String, Vec<u8>>, part: &str, from: &str, to: &str) {
    let text = String::from_utf8(files[part].clone()).unwrap();
    assert!(text.contains(from), "{part}: missing {from}");
    files.insert(part.into(), text.replace(from, to).into_bytes());
}

/// An original solid-blue PNG stands in for the document's independently stored preview.
fn preview() -> Vec<u8> {
    let mut bytes = Cursor::new(Vec::new());
    image::DynamicImage::ImageRgba8(image::RgbaImage::from_pixel(8, 4, image::Rgba([37, 99, 235, 255])))
        .write_to(&mut bytes, image::ImageFormat::Png)
        .unwrap();
    bytes.into_inner()
}

/// Adds an independent image fallback to an InkML fixture at the requested placement.
fn with_fallback(placement: &str, ink: &str) -> Vec<u8> {
    let mut files = unpack(&document_with_ink(placement, ink));
    let choice = r#"<mc:AlternateContent xmlns:mc="http://schemas.openxmlformats.org/markup-compatibility/2006"><mc:Choice xmlns:wpi="http://schemas.microsoft.com/office/word/2010/wordprocessingInk" Requires="wpi">"#;
    if matches!(placement, "group" | "canvas") {
        replace(&mut files, "word/document.xml", "<w14:contentPart ", &format!("{choice}<w14:contentPart "));
        replace(
            &mut files,
            "word/document.xml",
            "</w14:contentPart>",
            r#"</w14:contentPart></mc:Choice><mc:Fallback><pic:pic xmlns:pic="http://schemas.openxmlformats.org/drawingml/2006/picture"><pic:nvPicPr><pic:cNvPr id="9" name=""/><pic:cNvPicPr/></pic:nvPicPr><pic:blipFill><a:blip r:embed="preview"/></pic:blipFill><pic:spPr><a:xfrm><a:off x="254000" y="254000"/><a:ext cx="1371600" cy="736600"/></a:xfrm><a:prstGeom prst="rect"/></pic:spPr></pic:pic></mc:Fallback></mc:AlternateContent>"#,
        );
    } else {
        replace(&mut files, "word/document.xml", "<w:drawing>", &format!("{choice}<w:drawing>"));
        replace(
            &mut files,
            "word/document.xml",
            "</w:drawing>",
            r#"</w:drawing></mc:Choice><mc:Fallback><w:pict xmlns:v="urn:schemas-microsoft-com:vml"><v:shape style="width:216pt;height:116pt"><v:imagedata r:id="preview"/></v:shape></w:pict></mc:Fallback></mc:AlternateContent>"#,
        );
    }
    replace(
        &mut files,
        "word/_rels/document.xml.rels",
        "</Relationships>",
        r#"<Relationship Id="preview" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/image" Target="media/preview.png"/></Relationships>"#,
    );
    replace(&mut files, "[Content_Types].xml", "</Types>", r#"<Default Extension="png" ContentType="image/png"/></Types>"#);
    files.insert("word/media/preview.png".into(), preview());
    pack(files)
}

/// Collects document graphics recursively through grouped objects.
fn graphics(doc: &Document) -> Vec<&wordcraft_doc::graphic::Graphic> {
    /// Visits a graphic or its grouped children and appends each graphic found.
    fn collect<'a>(o: &'a InlineObject, out: &mut Vec<&'a wordcraft_doc::graphic::Graphic>) {
        match o {
            InlineObject::Graphic { graphic, .. } => out.push(graphic),
            InlineObject::Group { children, .. } => children.iter().for_each(|c| collect(&c.obj, out)),
            _ => {}
        }
    }
    let mut out = Vec::new();
    for b in &doc.body {
        if let Block::Para(p) = &**b {
            for o in &p.objects {
                collect(o, &mut out);
            }
        }
    }
    out
}

/// Finds the first graphic mutably in this synthetic document fixture.
fn first_graphic_mut(doc: &mut Document) -> &mut wordcraft_doc::graphic::Graphic {
    for block in &mut doc.body {
        let Block::Para(para) = std::sync::Arc::make_mut(block) else { continue };
        for object in &mut para.objects {
            if let InlineObject::Graphic { graphic, .. } = object {
                return std::sync::Arc::make_mut(graphic);
            }
        }
    }
    panic!("fixture must contain native ink");
}

/// Checks whether an ODT-reopened document contains a shape, image, graphic or group drawing.
fn has_drawing(objects: &[InlineObject]) -> bool {
    objects.iter().any(|object| match object {
        InlineObject::Image { .. } | InlineObject::Graphic { .. } | InlineObject::Shape { .. } => true,
        InlineObject::Group { children, .. } => children.iter().any(|child| has_drawing(std::slice::from_ref(&child.obj))),
        _ => false,
    })
}

/// Rejected ink must not consume the allowance used by later editable charts and diagrams.
#[test]
fn costly_native_ink_does_not_hide_later_charts_or_smart_art() {
    let expensive = format!(
        r#"<ink xmlns="http://www.w3.org/2003/InkML"><brush><brushProperty name="tip" value="rectangle"/></brush><trace>{}</trace></ink>"#,
        "0 0,1 1,".repeat(10_000)
    );
    let curved = format!(
        r#"<ink xmlns="http://www.w3.org/2003/InkML"><brush><brushProperty name="fitToCurve" value="true"/><brushProperty name="transparency" value="128"/></brush><trace>{}</trace></ink>"#,
        "0 0,1 1,".repeat(1_000)
    );
    let mut session = Session::new(Document::new());
    run(&mut session, "insert.chart", json!({"type":"column"}));
    session.sel.anchor = session.sel.focus.clone();
    run(&mut session, "insert.smartArt", json!({"layout":"process"}));
    let original = unpack(&crate::io::save_bytes("objects.docx", &session.doc).unwrap());
    for ink in ["invalid", INK, expensive.as_str(), curved.as_str()] {
        for nested in [false, true] {
            let mut files = original.clone();
            // A separate nested DOCX exercises propagation of both allowances out of altChunk.
            let bytes = with_fallback("inline", ink);
            let body = if nested {
                replace(
                    &mut files,
                    "word/_rels/document.xml.rels",
                    "</Relationships>",
                    r#"<Relationship Id="inkChunk" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/aFChunk" Target="chunk.docx"/></Relationships>"#,
                );
                files.insert("word/chunk.docx".into(), bytes);
                r#"<w:altChunk r:id="inkChunk"/>"#.to_string()
            } else {
                let source = unpack(&bytes);
                let xml = String::from_utf8(source["word/document.xml"].clone()).unwrap();
                let body = xml.split_once("<w:body>").unwrap().1.split_once("</w:body>").unwrap().0;
                // The copied fragment declares the namespaces inherited by the small fixture.
                let body = format!(
                    r#"<w:sdt xmlns:w14="http://schemas.microsoft.com/office/word/2010/wordml" xmlns:wp="http://schemas.openxmlformats.org/drawingml/2006/wordprocessingDrawing" xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main"><w:sdtContent>{body}</w:sdtContent></w:sdt>"#
                );
                let rels = String::from_utf8(source["word/_rels/document.xml.rels"].clone()).unwrap();
                let rels = rels.split_once('>').unwrap().1.trim_end_matches("</Relationships>");
                replace(&mut files, "word/_rels/document.xml.rels", "</Relationships>", &format!("{rels}</Relationships>"));
                replace(
                    &mut files,
                    "[Content_Types].xml",
                    "</Types>",
                    r#"<Override PartName="/word/ink/ink1.xml" ContentType="application/inkml+xml"/></Types>"#,
                );
                files.insert("word/ink/ink1.xml".into(), ink.as_bytes().to_vec());
                files.insert("word/media/preview.png".into(), preview());
                body
            };
            let later = r#"<w:p><w:r><w:contentPart r:id="laterInk"/></w:r></w:p>"#;
            replace(&mut files, "word/document.xml", "<w:body>", &format!("<w:body>{body}{later}"));
            replace(
                &mut files,
                "word/_rels/document.xml.rels",
                "</Relationships>",
                r#"<Relationship Id="laterInk" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/customXml" Target="ink/later.xml"/></Relationships>"#,
            );
            replace(
                &mut files,
                "[Content_Types].xml",
                "</Types>",
                r#"<Override PartName="/word/ink/later.xml" ContentType="application/inkml+xml"/></Types>"#,
            );
            files.insert("word/ink/later.xml".into(), INK.as_bytes().to_vec());
            let mut doc = crate::io::open_bytes("mixed.docx", &pack(files)).unwrap();
            for _ in 0..2 {
                let gs = graphics(&doc);
                assert!(
                    gs.iter()
                        .any(|g| g.kind == GraphicKind::Ink && matches!(g.items.first(), Some(wordcraft_doc::graphic::GraphicItem::Path { .. }))),
                    "later ink must still draw; nested={nested}"
                );
                if !nested {
                    // Source-part adoption from altChunks is an existing separate limitation;
                    // this fixture checks their shared work accounting, not that save path.
                    let native = gs.iter().find(|g| g.kind == GraphicKind::Ink).unwrap();
                    assert!(native.source.is_some());
                    assert!(doc.passthrough.values().any(|bytes| bytes.as_slice() == ink.as_bytes()));
                    if ink != INK {
                        assert!(
                            matches!(native.items.as_slice(), [wordcraft_doc::graphic::GraphicItem::Image { media, .. }] if doc.media.contains_key(media)),
                            "rejected ink must keep its picture"
                        );
                    }
                }
                let chart = gs.iter().find(|g| g.kind == GraphicKind::Chart).unwrap();
                let diagram = gs.iter().find(|g| g.kind == GraphicKind::Diagram).unwrap();
                assert!(!chart.items.is_empty() && chart.chart.is_some(), "chart lost; nested={nested}, ink bytes={}", ink.len());
                assert!(!diagram.items.is_empty() && diagram.smart_art.is_some(), "SmartArt lost; nested={nested}");
                let saved = crate::io::save_bytes("mixed.docx", &doc).unwrap();
                doc = crate::io::open_bytes("mixed.docx", &saved).unwrap();
            }
        }
    }
}

/// Initial nonuniform fitting and later canvas-member resizing compose before averaging widths.
#[test]
fn native_ink_canvas_member_widths_survive_nonuniform_resize_and_saves() {
    let ink = r#"<ink xmlns="http://www.w3.org/2003/InkML"><brush><brushProperty name="width" value="2" units="pt"/></brush><trace>0 0,8 8</trace><trace>4 4</trace></ink>"#;
    let mut files = unpack(&document_with_ink("canvas", ink));
    replace(&mut files, "word/document.xml", r#"cx="1371600" cy="736600""#, r#"cx="1270000" cy="635000""#);
    let mut session = Session::new(crate::io::open_bytes("resize.docx", &pack(files)).unwrap());
    run(&mut session, "select.objects", json!({"index":0}));
    run(&mut session, "canvas.member", json!({"index":0,"width":200,"height":50}));
    for _ in 0..3 {
        let layout = session.layout();
        let mut sizes = Vec::new();
        super::review::tap_and_line(&wordcraft_layout::display::page_display(&session.doc, &layout.pages[0], &Default::default()), &mut sizes);
        assert_eq!(sizes.len(), 2, "line and tap must survive: {sizes:?}");
        for (w, h) in sizes {
            assert!((w - 25.0).abs() < 1e-4 && (h - 25.0).abs() < 1e-4, "expected 2 * mean(20, 5), got {w} x {h}");
        }
        let saved = crate::io::save_bytes("resize.docx", &session.doc).unwrap();
        session = Session::new(crate::io::open_bytes("resize.docx", &saved).unwrap());
        assert!(session.doc.passthrough.values().any(|bytes| bytes.as_slice() == ink.as_bytes()));
    }
}

/// Native strokes with no usable picture fallback must not export the taps without their lines.
#[test]
fn native_ink_without_usable_fallback_never_exports_partial_strokes() {
    use wordcraft_doc::graphic::GraphicItem;

    let ink = r#"<ink xmlns="http://www.w3.org/2003/InkML"><trace>0 0,10 0</trace><trace>5 5</trace></ink>"#;
    let no_fallback = crate::io::open_bytes("no-fallback.docx", &document_with_ink("inline", ink)).unwrap();
    let mut missing_media = crate::io::open_bytes("missing-media.docx", &with_fallback("inline", ink)).unwrap();
    let fallback = graphics(&missing_media)[0].ink_fallback.clone().expect("fixture references a fallback");
    missing_media.media.remove(&fallback);

    for (label, doc) in [("no fallback", no_fallback), ("missing media", missing_media)] {
        let items = &graphics(&doc)[0].items;
        assert!(items.iter().any(|item| matches!(item, GraphicItem::Path { .. } | GraphicItem::InkOutline { .. })), "{label}: line missing");
        assert!(items.iter().any(|item| matches!(item, GraphicItem::Shape { .. })), "{label}: point missing");
        for name in ["ink-no-fallback.odt", "ink-no-fallback.ott"] {
            let bytes = crate::io::save_bytes(name, &doc).unwrap();
            let reopened = crate::io::open_bytes(name, &bytes).unwrap();
            assert!(
                !reopened.body.iter().any(|block| matches!(&**block, Block::Para(p) if has_drawing(&p.objects))),
                "{label}, {name}: a drawing was exported without its complete stroke"
            );
        }
    }
}

/// Ink whose items are already pictures remains convertible when no separate fallback is set.
#[test]
fn native_ink_image_items_still_export_without_a_separate_fallback() {
    use wordcraft_doc::graphic::GraphicItem;

    let mut doc = crate::io::open_bytes("image-items.docx", &with_fallback("inline", INK)).unwrap();
    let fallback = graphics(&doc)[0].ink_fallback.clone().expect("fixture references a fallback");
    first_graphic_mut(&mut doc).ink_fallback = None;
    first_graphic_mut(&mut doc).items = vec![GraphicItem::Image { rect: [0.0, 0.0, 20.0, 10.0], media: fallback }];
    for name in ["ink-image-items.odt", "ink-image-items.ott"] {
        let bytes = crate::io::save_bytes(name, &doc).unwrap();
        let reopened = crate::io::open_bytes(name, &bytes).unwrap();
        assert!(reopened.body.iter().any(|block| matches!(&**block, Block::Para(p) if has_preview(&reopened, &p.objects))), "{name}");
    }
}

/// Zero-pressure vectors paint nothing and must not resurrect their retained picture on export.
#[test]
fn zero_pressure_native_ink_stays_nonpainting_on_odt_and_ott_export() {
    for placement in ["inline", "floating", "group", "canvas"] {
        for pressure in ["0", "0.5"] {
            let ink = format!(
                r#"<ink xmlns="http://www.w3.org/2003/InkML"><traceFormat><channel name="X" units="pt"/><channel name="Y" units="pt"/><channel name="F"/></traceFormat><trace>0 0 {pressure},10 10 {pressure}</trace></ink>"#
            );
            let mut session = Session::new(crate::io::open_bytes("pressure.docx", &with_fallback(placement, &ink)).unwrap());
            let layout = session.layout();
            let page = &layout.pages[0];
            let painted = wordcraft_render::render_page(&session.doc, page, 1.0, &Default::default());
            let hidden = wordcraft_render::render_page(
                &session.doc,
                page,
                1.0,
                &wordcraft_render::RenderOptions {
                    display: wordcraft_layout::display::DisplayOptions { hide_ink: true, ..Default::default() },
                    ..Default::default()
                },
            );
            let visible = pressure != "0";
            assert_eq!(painted.pixels != hidden.pixels, visible, "{placement}: pressure={pressure}");
            for name in ["pressure.odt", "pressure.ott"] {
                let saved = crate::io::save_bytes(name, &session.doc).unwrap();
                let reopened = crate::io::open_bytes(name, &saved).unwrap();
                let has_picture = reopened.body.iter().any(|b| matches!(&**b, Block::Para(p) if has_preview(&reopened, &p.objects)));
                assert_eq!(has_picture, visible, "{placement}, {name}: pressure={pressure}");
            }
        }
    }
}

/// Hidden grouped and canvas ink remains hidden while visible sibling shapes export normally.
#[test]
fn hidden_native_ink_members_stay_hidden_on_export() {
    /// Finds any retained shape, including shapes nested in groups.
    fn has_shape(objects: &[InlineObject]) -> bool {
        objects.iter().any(|o| match o {
            InlineObject::Shape { .. } => true,
            InlineObject::Group { children, .. } => children.iter().any(|c| has_shape(std::slice::from_ref(&c.obj))),
            _ => false,
        })
    }
    for placement in ["group", "canvas"] {
        let mut files = unpack(&with_fallback(placement, INK));
        replace(
            &mut files,
            "word/document.xml",
            r#"<w14:contentPart r:id="ink">"#,
            r#"<w14:contentPart r:id="ink"><w14:nvContentPartPr><w14:cNvPr id="7" name="" hidden="1"/></w14:nvContentPartPr>"#,
        );
        let end = if placement == "group" { "</wpg:wgp>" } else { "</wpc:wpc>" };
        let shape = r#"<wps:wsp xmlns:wps="http://schemas.microsoft.com/office/word/2010/wordprocessingShape"><wps:cNvSpPr/><wps:spPr><a:xfrm><a:off x="0" y="0"/><a:ext cx="127000" cy="127000"/></a:xfrm><a:prstGeom prst="rect"/><a:solidFill><a:srgbClr val="FF0000"/></a:solidFill></wps:spPr><wps:bodyPr/></wps:wsp>"#;
        replace(&mut files, "word/document.xml", end, &format!("{shape}{end}"));
        let doc = crate::io::open_bytes("hidden.docx", &pack(files)).unwrap();
        for name in ["hidden.odt", "hidden.ott"] {
            let reopened = crate::io::open_bytes(name, &crate::io::save_bytes(name, &doc).unwrap()).unwrap();
            assert!(
                !reopened.body.iter().any(|b| matches!(&**b, Block::Para(p) if has_preview(&reopened, &p.objects))),
                "{placement}: hidden ink became visible"
            );
            assert!(reopened.body.iter().any(|b| matches!(&**b, Block::Para(p) if has_shape(&p.objects))), "visible sibling was lost");
        }
    }
}

/// Ink with no visible strokes does not restore its fallback picture in ODT/OTT exports.
#[test]
fn invisible_native_ink_does_not_resurrect_its_picture_on_export() {
    let ink = r#"<ink xmlns="http://www.w3.org/2003/InkML"><trace type="penUp">0 0,10 10</trace></ink>"#;
    let doc = crate::io::open_bytes("ink.docx", &with_fallback("inline", ink)).unwrap();
    for name in ["ink.odt", "ink.ott"] {
        let reopened = crate::io::open_bytes(name, &crate::io::save_bytes(name, &doc).unwrap()).unwrap();
        assert!(!reopened.body.iter().any(|b| matches!(&**b, Block::Para(p) if has_preview(&reopened, &p.objects))));
    }
}

/// Finds a retained preview image recursively and verifies its stored pixels.
fn has_preview(doc: &Document, objects: &[InlineObject]) -> bool {
    objects.iter().any(|o| match o {
        InlineObject::Image { media, w, h, .. } => *w > 0.0 && *h > 0.0 && doc.media.get(media).is_some_and(|data| data.as_slice() == preview()),
        InlineObject::Group { children, .. } => children.iter().any(|c| has_preview(doc, std::slice::from_ref(&c.obj))),
        _ => false,
    })
}

/// Independent preview images survive JSON adoption and ODT/OTT export with vector ink.
#[test]
fn native_ink_picture_fallback_survives_odt_and_ott_export() {
    for placement in ["inline", "floating", "group", "canvas"] {
        for ink in [INK, ADVANCED_INK] {
            let doc = crate::io::open_bytes("ink.docx", &with_fallback(placement, ink)).unwrap();
            let graphic = graphics(&doc)[0];
            assert!(!graphic.items.is_empty());
            assert!(graphic.items.iter().all(|i| !matches!(i, wordcraft_doc::graphic::GraphicItem::Image { .. })));
            // Recovery and copy/adoption must retain the independently stored fallback, too.
            let doc: Document = serde_json::from_slice(&serde_json::to_vec(&doc).unwrap()).unwrap();
            let mut destination = Document::new();
            for key in doc.media.keys() {
                destination.media.insert(key.clone(), std::sync::Arc::new(vec![1, 2, 3]));
            }
            let blocks = destination.adopt_blocks(doc, false, &mut 0);
            destination.body = blocks.into_iter().map(std::sync::Arc::new).collect();
            for name in ["ink.odt", "ink.ott"] {
                let bytes = crate::io::save_bytes(name, &destination).unwrap();
                let reopened = crate::io::open_bytes(name, &bytes).unwrap();
                assert!(
                    reopened.body.iter().any(|b| matches!(&**b, Block::Para(p) if has_preview(&reopened, &p.objects))),
                    "{placement}, {name}: preview disappeared"
                );
            }
        }
    }
}
