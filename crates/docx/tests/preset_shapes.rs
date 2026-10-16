//! More preset shapes and picture fills (#1534): the hexagon, octagon, pentagon, parallelogram,
//! trapezoid, plus, chevron and decagon presets (ECMA-376 Part 1 §20.1.10.56 `ST_ShapeType`), and
//! a shape filled with a picture (`a:blipFill` in `wps:spPr`, §20.1.8.14). Fixtures are
//! hand-written.

use std::io::{Read, Write};

use wordcraft_doc::para::{Float, ShapeKind};
use wordcraft_doc::props::{CharProps, Rgb};
use wordcraft_doc::{Document, InlineObject, Paragraph, para_block};

const NEW: [(ShapeKind, &str); 8] = [
    (ShapeKind::Hexagon, "hexagon"),
    (ShapeKind::Octagon, "octagon"),
    (ShapeKind::Pentagon, "pentagon"),
    (ShapeKind::Parallelogram, "parallelogram"),
    (ShapeKind::Trapezoid, "trapezoid"),
    (ShapeKind::Plus, "plus"),
    (ShapeKind::Chevron, "chevron"),
    (ShapeKind::Decagon, "decagon"),
];

fn zip(entries: &[(&str, &[u8])]) -> Vec<u8> {
    let mut zw = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    for (name, bytes) in entries {
        zw.start_file(*name, zip::write::SimpleFileOptions::default()).unwrap();
        zw.write_all(bytes).unwrap();
    }
    zw.finish().unwrap().into_inner()
}

fn png() -> Vec<u8> {
    let img = image::RgbaImage::from_fn(8, 8, |x, _| if x < 4 { image::Rgba([200, 30, 30, 255]) } else { image::Rgba([30, 30, 200, 255]) });
    let mut b = Vec::new();
    image::DynamicImage::ImageRgba8(img).write_to(&mut std::io::Cursor::new(&mut b), image::ImageFormat::Png).unwrap();
    b
}

fn shape(kind: ShapeKind, fill: Option<Rgb>, picture: Option<String>) -> InlineObject {
    InlineObject::Shape {
        kind,
        w: 100.0,
        h: 60.0,
        fill,
        stroke: Some(Rgb(0x0E, 0x40, 0x5A)),
        stroke_width: 1.0,
        float: Float::default(),
        story: None,
        freeform: None,
        effects: Default::default(),
        extra: wordcraft_doc::connector::ShapeExtra { picture, ..Default::default() },
    }
}

fn doc_of(objs: Vec<InlineObject>) -> Document {
    let mut p = Paragraph::with_text("x", CharProps::default());
    for o in objs {
        let n = p.len();
        p.insert_object(n, o, &CharProps::default()).unwrap();
    }
    let mut d = Document::new();
    d.body = vec![para_block(p)];
    d
}

fn objects(d: &Document) -> Vec<InlineObject> {
    d.body.iter().filter_map(|b| b.as_para()).flat_map(|p| p.objects.iter().cloned()).collect()
}

fn part(bytes: &[u8], name: &str) -> String {
    let mut zip = zip::ZipArchive::new(std::io::Cursor::new(bytes)).unwrap();
    let mut xml = String::new();
    zip.by_name(name).unwrap().read_to_string(&mut xml).unwrap();
    xml
}

#[test]
fn new_presets_round_trip() {
    let d = doc_of(NEW.iter().map(|(k, _)| shape(*k, Some(Rgb(0x15, 0x60, 0x82)), None)).collect());
    let bytes = wordcraft_docx::write(&d).unwrap();
    let xml = part(&bytes, "word/document.xml");
    for (_, prst) in NEW {
        assert!(xml.contains(&format!(r#"<a:prstGeom prst="{prst}">"#)), "{prst}");
    }
    let r = wordcraft_docx::read(&bytes).unwrap();
    let kinds: Vec<ShapeKind> =
        objects(&r).iter().filter_map(|o| if let InlineObject::Shape { kind, .. } = o { Some(*kind) } else { None }).collect();
    assert_eq!(kinds, NEW.map(|(k, _)| k).to_vec());
}

/// A hand-written package: a hexagon whose `wps:spPr` has a stretched `a:blipFill`.
#[test]
fn picture_fill_reads_from_blip_fill() {
    let rels = r#"<?xml version="1.0" encoding="UTF-8"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="word/document.xml"/></Relationships>"#;
    let doc_rels = r#"<?xml version="1.0" encoding="UTF-8"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId7" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/image" Target="media/fill.png"/></Relationships>"#;
    let wsp = |fill: &str| {
        format!(
            r#"<w:r><w:drawing><wp:inline><wp:extent cx="1270000" cy="762000"/><wp:docPr id="1" name="Hexagon 1"/><a:graphic><a:graphicData uri="http://schemas.microsoft.com/office/word/2010/wordprocessingShape"><wps:wsp><wps:cNvSpPr/><wps:spPr><a:xfrm><a:off x="0" y="0"/><a:ext cx="1270000" cy="762000"/></a:xfrm><a:prstGeom prst="hexagon"><a:avLst/></a:prstGeom>{fill}</wps:spPr><wps:bodyPr/></wps:wsp></a:graphicData></a:graphic></wp:inline></w:drawing></w:r>"#
        )
    };
    let stretched = wsp(r#"<a:blipFill><a:blip r:embed="rId7"/><a:stretch><a:fillRect/></a:stretch></a:blipFill>"#);
    // Tiled pictures load stretched; a fill whose picture is missing is no picture.
    let tiled = wsp(r#"<a:blipFill rotWithShape="1"><a:blip r:embed="rId7"/><a:tile tx="0" ty="0" sx="100000" sy="100000"/></a:blipFill>"#);
    let missing = wsp(r#"<a:blipFill><a:blip r:embed="rId99"/><a:stretch><a:fillRect/></a:stretch></a:blipFill>"#);
    let body = format!("<w:p>{stretched}{tiled}{missing}</w:p>");
    let doc = format!(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships" xmlns:wp="http://schemas.openxmlformats.org/drawingml/2006/wordprocessingDrawing" xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" xmlns:wps="http://schemas.microsoft.com/office/word/2010/wordprocessingShape"><w:body>{body}</w:body></w:document>"#
    );
    let png = png();
    let bytes = zip(&[
        ("_rels/.rels", rels.as_bytes()),
        ("word/document.xml", doc.as_bytes()),
        ("word/_rels/document.xml.rels", doc_rels.as_bytes()),
        ("word/media/fill.png", &png),
    ]);
    let d = wordcraft_docx::read(&bytes).unwrap();
    let fills: Vec<(ShapeKind, Option<Rgb>, Option<Vec<u8>>)> = objects(&d)
        .iter()
        .filter_map(|o| match o {
            InlineObject::Shape { kind, fill, extra, .. } => {
                Some((*kind, *fill, extra.picture.as_ref().and_then(|k| d.media.get(k)).map(|b| b.to_vec())))
            }
            _ => None,
        })
        .collect();
    assert_eq!(
        fills,
        vec![(ShapeKind::Hexagon, None, Some(png.clone())), (ShapeKind::Hexagon, None, Some(png.clone())), (ShapeKind::Hexagon, None, None)]
    );
}

#[test]
fn picture_fill_round_trips() {
    let png = png();
    let mut d = doc_of(vec![]);
    let key = d.add_media(png.clone(), "png");
    let mut p = Paragraph::with_text("x", CharProps::default());
    p.insert_object(1, shape(ShapeKind::Hexagon, None, Some(key)), &CharProps::default()).unwrap();
    d.body = vec![para_block(p)];
    let bytes = wordcraft_docx::write(&d).unwrap();
    let xml = part(&bytes, "word/document.xml");
    assert!(xml.contains("<a:blipFill"), "{xml}");
    assert!(xml.contains("<a:stretch><a:fillRect/></a:stretch>"), "{xml}");
    let r = wordcraft_docx::read(&bytes).unwrap();
    let got = objects(&r).into_iter().find_map(|o| match o {
        InlineObject::Shape { kind, fill, extra, .. } => Some((kind, fill, extra.picture.and_then(|k| r.media.get(&k).map(|b| b.to_vec())))),
        _ => None,
    });
    assert_eq!(got, Some((ShapeKind::Hexagon, None, Some(png))));
}

/// #1534 review: a freeform (`a:custGeom`) filled with a picture reads as a freeform with its
/// picture fill, and saving keeps both.
#[test]
fn freeform_picture_fill_reads_and_round_trips() {
    let rels = r#"<?xml version="1.0" encoding="UTF-8"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="word/document.xml"/></Relationships>"#;
    let doc_rels = r#"<?xml version="1.0" encoding="UTF-8"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId7" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/image" Target="media/fill.png"/></Relationships>"#;
    let body = r#"<w:p><w:r><w:drawing><wp:inline><wp:extent cx="1270000" cy="762000"/><wp:docPr id="1" name="Freeform 1"/><a:graphic><a:graphicData uri="http://schemas.microsoft.com/office/word/2010/wordprocessingShape"><wps:wsp><wps:cNvSpPr/><wps:spPr><a:xfrm><a:off x="0" y="0"/><a:ext cx="1270000" cy="762000"/></a:xfrm><a:custGeom><a:avLst/><a:gdLst/><a:ahLst/><a:cxnLst/><a:rect l="0" t="0" r="r" b="b"/><a:pathLst><a:path w="100" h="60"><a:moveTo><a:pt x="0" y="0"/></a:moveTo><a:lnTo><a:pt x="100" y="0"/></a:lnTo><a:lnTo><a:pt x="0" y="60"/></a:lnTo><a:close/></a:path></a:pathLst></a:custGeom><a:blipFill><a:blip r:embed="rId7"/><a:stretch><a:fillRect/></a:stretch></a:blipFill></wps:spPr><wps:bodyPr/></wps:wsp></a:graphicData></a:graphic></wp:inline></w:drawing></w:r></w:p>"#;
    let doc = format!(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships" xmlns:wp="http://schemas.openxmlformats.org/drawingml/2006/wordprocessingDrawing" xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" xmlns:wps="http://schemas.microsoft.com/office/word/2010/wordprocessingShape"><w:body>{body}</w:body></w:document>"#
    );
    let png = png();
    let bytes = zip(&[
        ("_rels/.rels", rels.as_bytes()),
        ("word/document.xml", doc.as_bytes()),
        ("word/_rels/document.xml.rels", doc_rels.as_bytes()),
        ("word/media/fill.png", &png),
    ]);
    let check = |d: &Document| {
        let got = objects(d).into_iter().find_map(|o| match o {
            InlineObject::Shape { kind, freeform, extra, .. } => {
                Some((kind, freeform.map(|f| f.paths.len()), extra.picture.and_then(|k| d.media.get(&k).map(|b| b.to_vec()))))
            }
            _ => None,
        });
        assert_eq!(got, Some((ShapeKind::Freeform, Some(1), Some(png.clone()))));
    };
    let d = wordcraft_docx::read(&bytes).unwrap();
    check(&d);
    check(&wordcraft_docx::read(&wordcraft_docx::write(&d).unwrap()).unwrap());
}
