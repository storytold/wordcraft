//! Original nested-picture controls. No native output or external assets are embedded.
use std::{io::Write, sync::Arc};
use wordcraft_doc::para::{Anchor, CanvasStyle, Float, GroupChild, ShapeKind, Wrap};
use wordcraft_doc::props::Rgb;
use wordcraft_doc::{Document, InlineObject, Paragraph, PartKind, Pos, StoryRef, TextBody, para_block};

// Original solid RGB(0,72,192), generated in code for these controls.
const BLUE_PNG: &[u8] = &[
    137, 80, 78, 71, 13, 10, 26, 10, 0, 0, 0, 13, 73, 72, 68, 82, 0, 0, 0, 64, 0, 0, 0, 16, 8, 2, 0, 0, 0, 41, 133, 238, 126, 0, 0, 0, 54, 73, 68,
    65, 84, 120, 156, 237, 207, 65, 9, 0, 48, 12, 192, 192, 202, 155, 236, 73, 171, 136, 61, 142, 65, 32, 2, 46, 51, 231, 254, 157, 23, 52, 160, 5,
    13, 104, 65, 3, 90, 208, 128, 22, 52, 160, 5, 13, 104, 65, 3, 90, 240, 214, 2, 129, 33, 32, 61, 59, 114, 68, 120, 0, 0, 0, 0, 73, 69, 78, 68,
    174, 66, 96, 130,
];
const INLINE_XML: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships" xmlns:wp="http://schemas.openxmlformats.org/drawingml/2006/wordprocessingDrawing" xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" xmlns:c="http://schemas.openxmlformats.org/drawingml/2006/chart" xmlns:pic="http://schemas.openxmlformats.org/drawingml/2006/picture" xmlns:mc="http://schemas.openxmlformats.org/markup-compatibility/2006" xmlns:w14="http://schemas.microsoft.com/office/word/2010/wordml" xmlns:wpi="http://schemas.microsoft.com/office/word/2010/wordprocessingInk" xmlns:w15="http://schemas.microsoft.com/office/word/2012/wordml" xmlns:wp14="http://schemas.microsoft.com/office/word/2010/wordprocessingDrawing" xmlns:wps="http://schemas.microsoft.com/office/word/2010/wordprocessingShape" xmlns:wpg="http://schemas.microsoft.com/office/word/2010/wordprocessingGroup" xmlns:wpc="http://schemas.microsoft.com/office/word/2010/wordprocessingCanvas" xmlns:dgm="http://schemas.openxmlformats.org/drawingml/2006/diagram" xmlns:dsp="http://schemas.microsoft.com/office/drawing/2008/diagram" xmlns:m="http://schemas.openxmlformats.org/officeDocument/2006/math" xmlns:v="urn:schemas-microsoft-com:vml" xmlns:o="urn:schemas-microsoft-com:office:office" xmlns:w10="urn:schemas-microsoft-com:office:word" xmlns:cx="http://schemas.microsoft.com/office/drawing/2014/chartex" xmlns:cx1="http://schemas.microsoft.com/office/drawing/2015/9/8/chartex" xmlns:cx2="http://schemas.microsoft.com/office/drawing/2015/10/21/chartex" xmlns:cx3="http://schemas.microsoft.com/office/drawing/2016/5/9/chartex" xmlns:cx4="http://schemas.microsoft.com/office/drawing/2016/5/10/chartex" xmlns:cx5="http://schemas.microsoft.com/office/drawing/2016/5/11/chartex" xmlns:cx6="http://schemas.microsoft.com/office/drawing/2016/5/12/chartex" xmlns:cx7="http://schemas.microsoft.com/office/drawing/2016/5/13/chartex" xmlns:cx8="http://schemas.microsoft.com/office/drawing/2016/5/14/chartex" mc:Ignorable="w14 w15 wp14"><w:body><w:p><w:pPr><w:keepNext w:val="0"/><w:keepLines w:val="0"/><w:pageBreakBefore w:val="0"/><w:widowControl w:val="0"/><w:spacing w:before="0" w:after="0" w:line="320" w:lineRule="exact"/><w:rPr><w:rFonts w:ascii="Arial" w:hAnsi="Arial"/><w:sz w:val="20"/></w:rPr></w:pPr><w:r><w:rPr><w:rFonts w:ascii="Arial" w:hAnsi="Arial"/><w:sz w:val="20"/></w:rPr><w:t xml:space="preserve">2533 Plain | red parent 180 pt wide | saved height 80 pt</w:t></w:r><w:r><w:rPr><w:rFonts w:ascii="Arial" w:hAnsi="Arial"/><w:sz w:val="20"/></w:rPr><w:drawing><wp:anchor distT="0" distB="0" distL="0" distR="0" simplePos="0" relativeHeight="251658241" behindDoc="0" locked="0" layoutInCell="1" allowOverlap="1"><wp:simplePos x="0" y="0"/><wp:positionH relativeFrom="page"><wp:posOffset>914400</wp:posOffset></wp:positionH><wp:positionV relativeFrom="page"><wp:posOffset>1270000</wp:posOffset></wp:positionV><wp:extent cx="2286000" cy="1016000"/><wp:effectExtent l="0" t="0" r="0" b="0"/><wp:wrapNone/><wp:docPr id="1" name="Shape 1" descr=""/><wp:cNvGraphicFramePr/><a:graphic><a:graphicData uri="http://schemas.microsoft.com/office/word/2010/wordprocessingShape"><wps:wsp><wps:cNvSpPr txBox="1"/><wps:spPr><a:xfrm><a:off x="0" y="0"/><a:ext cx="2286000" cy="1016000"/></a:xfrm><a:prstGeom prst="rect"><a:avLst/></a:prstGeom><a:solidFill><a:srgbClr val="F0F8F0"/></a:solidFill><a:ln w="12700"><a:solidFill><a:srgbClr val="C00000"/></a:solidFill></a:ln></wps:spPr><wps:txbx><w:txbxContent><w:p><w:pPr><w:keepNext w:val="0"/><w:keepLines w:val="0"/><w:widowControl w:val="0"/><w:spacing w:before="0" w:after="0" w:line="320" w:lineRule="exact"/><w:rPr><w:rFonts w:ascii="Arial" w:hAnsi="Arial"/><w:sz w:val="20"/></w:rPr></w:pPr><w:r><w:drawing><wp:inline distT="0" distB="0" distL="0" distR="0"><wp:extent cx="508000" cy="127000"/><wp:effectExtent l="0" t="0" r="0" b="0"/><wp:docPr id="2" name="Original blue rectangle"/><wp:cNvGraphicFramePr><a:graphicFrameLocks noChangeAspect="1"/></wp:cNvGraphicFramePr><a:graphic><a:graphicData uri="http://schemas.openxmlformats.org/drawingml/2006/picture"><pic:pic><pic:nvPicPr><pic:cNvPr id="0" name="original-blue.png"/><pic:cNvPicPr/></pic:nvPicPr><pic:blipFill><a:blip r:embed="rIdIsolationRect"/><a:stretch><a:fillRect/></a:stretch></pic:blipFill><pic:spPr><a:xfrm><a:off x="0" y="0"/><a:ext cx="508000" cy="127000"/></a:xfrm><a:prstGeom prst="rect"><a:avLst/></a:prstGeom></pic:spPr></pic:pic></a:graphicData></a:graphic></wp:inline></w:drawing></w:r><w:r><w:rPr><w:rFonts w:ascii="Arial" w:hAnsi="Arial"/><w:sz w:val="20"/></w:rPr><w:t xml:space="preserve">Parent</w:t></w:r></w:p></w:txbxContent></wps:txbx><wps:bodyPr lIns="0" tIns="0" rIns="0" bIns="0"><a:spAutoFit/></wps:bodyPr></wps:wsp></a:graphicData></a:graphic></wp:anchor></w:drawing></w:r></w:p><w:sectPr><w:type w:val="nextPage"/><w:pgSz w:w="12240" w:h="15840"/><w:pgMar w:top="1440" w:right="1440" w:bottom="1440" w:left="1440" w:header="720" w:footer="720" w:gutter="0"/><w:cols w:space="720" w:num="1"/></w:sectPr></w:body></w:document>"#;

fn picture() -> InlineObject {
    InlineObject::Image {
        media: "original-blue.png".into(),
        w: 40.0,
        h: 10.0,
        alt: String::new(),
        float: Default::default(),
        crop: [0.0; 4],
        ole: None,
    }
}
fn model_box() -> Document {
    let mut doc = Document::new();
    doc.media.insert("original-blue.png".into(), Arc::new(BLUE_PNG.to_vec()));
    let id = doc.add_part(PartKind::TextBox, vec![para_block(Paragraph::with_text("Parent", Default::default()))]);
    doc.parts.get_mut(&id).unwrap().body = TextBody { auto_fit: true, inset: Some([0; 4]), ..Default::default() };
    doc.insert_object(&Pos { story: StoryRef::Part(id), path: wordcraft_doc::Path(vec![0]), off: 0 }, picture(), &Default::default()).unwrap();
    let parent = InlineObject::Shape {
        kind: ShapeKind::TextBox,
        w: 180.0,
        h: 80.0,
        fill: Some(wordcraft_doc::fill::Fill::solid(Rgb(240, 248, 240))),
        stroke: Some(Rgb(192, 0, 0)),
        stroke_width: 1.0,
        float: Float { wrap: Wrap::InFrontOfText, h_rel: Anchor::Page, v_rel: Anchor::Page, x: 72.0, y: 100.0, ..Default::default() },
        story: Some(id),
        freeform: None,
        effects: Default::default(),
        extra: Default::default(),
    };
    doc.insert_object(&Pos::body(0, 0), parent, &Default::default()).unwrap();
    doc
}
fn check_box(doc: &Document) {
    let objects = &doc.body[0].as_para().unwrap().objects;
    assert_eq!(objects.len(), 1);
    let InlineObject::Shape { kind, w, h, float, story: Some(id), stroke, stroke_width, .. } = &objects[0] else {
        panic!("outer textbox lost: {:?}", objects[0]);
    };
    assert_eq!((*kind, *w, *h, *stroke, *stroke_width), (ShapeKind::TextBox, 180.0, 80.0, Some(Rgb(192, 0, 0)), 1.0));
    assert_eq!((float.wrap, float.h_rel, float.v_rel, float.x, float.y), (Wrap::InFrontOfText, Anchor::Page, Anchor::Page, 72.0, 100.0));
    let part = &doc.parts[id];
    assert_eq!(part.kind, PartKind::TextBox);
    assert!(part.body.auto_fit);
    assert_eq!(part.body.inset, Some([0; 4]));
    assert_eq!(doc.parts.len(), 1);
    let para = part.blocks[0].as_para().unwrap();
    assert_eq!(para.text, "\u{fffc}Parent");
    assert_eq!(para.objects.len(), 1);
    let InlineObject::Image { w, h, float, media, .. } = &para.objects[0] else {
        panic!("inline picture lost");
    };
    assert_eq!((*w, *h, float.wrap), (40.0, 10.0, Wrap::Inline));
    assert_eq!(doc.media[media].as_slice(), BLUE_PNG);
}
fn package(xml: &str) -> Vec<u8> {
    let mut zip = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    let rels = br#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="word/document.xml"/></Relationships>"#;
    let docrels = br#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rIdIsolationRect" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/image" Target="media/isolation-blue.png"/></Relationships>"#;
    let types = br#"<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="png" ContentType="image/png"/><Override PartName="/word/document.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml"/></Types>"#;
    for (name, bytes) in [
        ("_rels/.rels", rels.as_slice()),
        ("[Content_Types].xml", types.as_slice()),
        ("word/document.xml", xml.as_bytes()),
        ("word/_rels/document.xml.rels", docrels.as_slice()),
        ("word/media/isolation-blue.png", BLUE_PNG),
    ] {
        zip.start_file(name, zip::write::SimpleFileOptions::default()).unwrap();
        zip.write_all(bytes).unwrap();
    }
    zip.finish().unwrap().into_inner()
}
#[test]
fn model_nested_picture_roundtrip() {
    let doc = model_box();
    check_box(&doc);
    let bytes = wordcraft_docx::write(&doc).unwrap();
    check_box(&wordcraft_docx::read(&bytes).unwrap());
}
#[test]
fn original_inline_xml_keeps_box() {
    let doc = wordcraft_docx::read(&package(INLINE_XML)).unwrap();
    check_box(&doc);
    check_box(&wordcraft_docx::read(&wordcraft_docx::write(&doc).unwrap()).unwrap());
}
#[test]
fn picture_group_canvas_keep_kind() {
    let group = |canvas| InlineObject::Group {
        w: 100.0,
        h: 50.0,
        float: Default::default(),
        ch_w: 100.0,
        ch_h: 50.0,
        children: vec![GroupChild { x: 3.0, y: 4.0, obj: picture() }],
        canvas,
        picture_fill: false,
    };
    for original in [picture(), group(None), group(Some(CanvasStyle::default()))] {
        let mut doc = Document::new();
        doc.media.insert("original-blue.png".into(), Arc::new(BLUE_PNG.to_vec()));
        doc.insert_object(&Pos::body(0, 0), original.clone(), &Default::default()).unwrap();
        let got = wordcraft_docx::read(&wordcraft_docx::write(&doc).unwrap()).unwrap();
        let object = &got.body[0].as_para().unwrap().objects[0];
        match (&original, object) {
            (InlineObject::Image { .. }, InlineObject::Image { w, h, media, .. }) => {
                assert_eq!((*w, *h), (40.0, 10.0));
                assert_eq!(got.media[media].as_slice(), BLUE_PNG);
            }
            (InlineObject::Group { canvas: expected, .. }, InlineObject::Group { w, h, children, canvas, .. }) => {
                assert_eq!((*w, *h), (100.0, 50.0));
                assert_eq!(canvas.is_some(), expected.is_some());
                assert_eq!(children.len(), 1);
                let child = &children[0];
                assert_eq!((child.x, child.y), (3.0, 4.0));
                let InlineObject::Image { w, h, media, .. } = &child.obj else {
                    panic!("picture member changed kind");
                };
                assert_eq!((*w, *h), (40.0, 10.0));
                assert_eq!(got.media[media].as_slice(), BLUE_PNG);
            }
            _ => panic!("payload changed kind: {object:?}"),
        }
        assert!(got.parts.is_empty());
    }
}

#[test]
fn own_picture_fill_keeps_image() {
    // Keep the existing WPS picture-fill fallback, distinct from a nested picture.
    let start = INLINE_XML.find("<wps:spPr>").unwrap();
    let end = INLINE_XML.find("</wps:wsp>").unwrap();
    let payload = r#"<wps:spPr><a:xfrm><a:off x="0" y="0"/><a:ext cx="2286000" cy="1016000"/></a:xfrm><a:prstGeom prst="rect"><a:avLst/></a:prstGeom><a:blipFill><a:blip r:embed="rIdIsolationRect"/><a:stretch><a:fillRect/></a:stretch></a:blipFill></wps:spPr><wps:bodyPr/>"#;
    let xml = format!("{}{}{}", &INLINE_XML[..start], payload, &INLINE_XML[end..]);
    let doc = wordcraft_docx::read(&package(&xml)).unwrap();
    assert!(doc.parts.is_empty());
    let objects = &doc.body[0].as_para().unwrap().objects;
    assert_eq!(objects.len(), 1);
    let InlineObject::Image { w, h, media, float, .. } = &objects[0] else {
        panic!("own picture fill lost: {:?}", objects[0]);
    };
    assert_eq!((*w, *h), (180.0, 80.0));
    assert_eq!((float.wrap, float.x, float.y), (Wrap::InFrontOfText, 72.0, 100.0));
    assert_eq!(doc.media[media].as_slice(), BLUE_PNG);
}
