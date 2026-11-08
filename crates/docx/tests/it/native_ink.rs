//! Integration regressions for native ink decoding, fallbacks, package preservation, and budgets.

use super::*;
use wordcraft_doc::graphic::{Graphic, GraphicItem, PathSeg};

#[path = "native_ink/xml.rs"]
mod xml;

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
        INK_XML.replace("</brush>", r#"<brushProperty name="rasterOp" value="unsupported"/></brush>"#),
        INK_XML.replace("</context>", "<canvasTransform/></context>"),
        "<not-ink>broken".to_string(),
        r#"<ink xmlns="http://www.w3.org/2003/InkML"><context><canvasTransform><mapping type="product"><mapping type="table"><bind source="X" column="1"/><bind target="X" column="2"/><table>F T,T F</table></mapping><mapping type="affine"><bind source="Y" column="1"/><bind target="Y" column="1"/><affine>1 0</affine></mapping></mapping></canvasTransform></context><trace>0 0,1 1</trace><trace>.5 0</trace></ink>"#.to_string(),
        r#"<ink xmlns="http://www.w3.org/2003/InkML"><traceFormat><channel name="X" type="integer"/><channel name="Y"/></traceFormat><context><canvas><traceFormat><channel name="X" type="integer" min="0" max="2"/><channel name="Y"/></traceFormat></canvas><canvasTransform><mapping type="unknown"/><mapping type="product"><mapping type="affine"><bind source="X" column="1"/><bind target="X" column="1"/><affine>.6 0</affine></mapping><mapping type="affine"><bind source="Y" column="1"/><bind target="Y" column="1"/><affine>1 0</affine></mapping></mapping></canvasTransform></context><trace>0 0,0 1</trace><trace>1 1</trace></ink>"#.to_string(),
        r#"<ink xmlns="http://www.w3.org/2003/InkML"><traceFormat><channel name="X"/><channel name="Y"/><channel name="F"/></traceFormat><trace>0 0 .5,1 1 1</trace><trace>2 2 1.1</trace></ink>"#.to_string(),
        r#"<ink xmlns="http://www.w3.org/2003/InkML"><context><canvasTransform><mapping type="unknown"/><mapping type="product"><mapping type="mathml" xmlns:m="http://www.w3.org/1998/Math/MathML"><bind source="X" variable="x"/><bind target="X"/><m:math><m:apply><m:root/><m:ci>x</m:ci></m:apply></m:math></mapping><mapping type="affine"><bind source="Y" column="1"/><bind target="Y" column="1"/><affine>1 0</affine></mapping></mapping></canvasTransform></context><trace>0 0,1 1</trace><trace>-1e-100 0,2 2</trace></ink>"#.to_string(),
        r#"<ink xmlns="http://www.w3.org/2003/InkML"><traceFormat><channel name="X" type="integer"/><channel name="Y" type="integer"/></traceFormat><context><canvas><traceFormat><channel name="X" type="integer"/><channel name="Y" type="integer"/></traceFormat></canvas><canvasTransform><mapping type="unknown"/><mapping type="affine"><affine>0.5 0 0,0 1 0</affine></mapping></canvasTransform></context><trace>0 0,1 1</trace></ink>"#.to_string(),
        r#"<ink xmlns="http://www.w3.org/2003/InkML"><trace>0 0,10 10</trace><trace type="indeterminate">0 10,10 0</trace></ink>"#.to_string(),
        r#"<ink xmlns="http://www.w3.org/2003/InkML"><trace type="indeterminate">0 0,10 10</trace></ink>"#.to_string(),
        r#"<ink xmlns="http://www.w3.org/2003/InkML"><traceFormat><channel name="X"/><channel name="Y"/><intermittentChannels><channel name="S" type="boolean"/></intermittentChannels></traceFormat><trace>0 0 T,5 5 ?,10 10 T</trace></ink>"#.to_string(),
        r#"<ink xmlns="http://www.w3.org/2003/InkML"><traceFormat><channel name="X"/><channel name="Y"/><intermittentChannels><channel name="F"/></intermittentChannels></traceFormat><trace>0 0 .5,5 5 ?,10 10 .5</trace></ink>"#.to_string(),
        r#"<ink xmlns="http://www.w3.org/2003/InkML"><traceFormat><channel name="X"/><channel name="Y"/><channel name="F" units="N"/></traceFormat><trace>0 0 1,10 10 100</trace></ink>"#.to_string(),
        r##"<ink xmlns="http://www.w3.org/2003/InkML"><definitions><brush xml:id="curve"><brushProperty name="fitToCurve" value="true"/></brush></definitions><trace xml:id="a" continuation="begin">0 0,10 0</trace><trace brushRef="#curve" continuation="end" priorRef="#a">10 0,20 0</trace></ink>"##.to_string(),
    ];
    for ink in variants {
        for (bytes, grouped) in frames(true) {
            let doc = wordcraft_docx::read(&replace_ink(&bytes, &ink)).unwrap();
            let mut doc: Document = serde_json::from_slice(&serde_json::to_vec(&doc).unwrap()).unwrap();
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

/// Advanced nibs, mapped coordinates, sparse channels and continuation traces keep vectors,
/// source bytes and fallback images through JSON and repeated DOCX round trips.
#[test]
fn advanced_native_ink_roundtrips_json_and_all_drawing_frames() {
    let variants = [
        r#"<traceFormat><channel name="X"/><channel name="Y"/><channel name="L" type="boolean"/></traceFormat><context><canvas><traceFormat><channel name="X"/><channel name="Y"/><channel name="S" type="boolean"/></traceFormat></canvas><canvasTransform><mapping type="product"><mapping type="affine"><bind source="X" column="1"/><bind source="Y" column="2"/><bind target="X" column="1"/><bind target="Y" column="2"/><affine>1 0 0,0 1 0</affine></mapping><mapping type="table"><bind source="L" column="1"/><bind target="S" column="2"/><table>F T,T F</table></mapping></mapping></canvasTransform></context><trace>0 0 F,10 10 F,100 100 T,20 10 F,30 0 F</trace>"#,
        r#"<context><canvas><traceFormat><channel name="X" type="integer" min="0" max="3"/><channel name="Y"/></traceFormat></canvas><canvasTransform><mapping type="unknown"/><mapping type="product"><mapping type="table"><bind source="X" column="1"/><bind target="X" column="2"/><table interpolation="floor">0 2,1 0,2 3,3 1</table></mapping><mapping type="affine"><bind source="Y" column="1"/><bind target="Y" column="1"/><affine>1 0</affine></mapping></mapping></canvasTransform></context><trace>2 0,1 0,1 2</trace>"#,
        r#"<traceFormat><channel name="X"/><channel name="Y"/><channel name="BW"/><channel name="F"/></traceFormat><trace>0 0 1 .5,25.4 12.7 2 1</trace>"#,
        r##"<brush><brushProperty name="color" value="#2563EB"/><brushProperty name="width" value="4"/><brushProperty name="height" value="8"/><brushProperty name="tip" value="rectangle"/><brushProperty name="transparency" value="128"/></brush><trace>0 0,10 10</trace>"##,
        r##"<context><canvasTransform><mapping type="affine"><affine>2 0 0,0 3 0</affine></mapping></canvasTransform></context><trace xml:id="a" continuation="begin">0 0,10 10</trace><trace continuation="end" priorRef="#a">20 10,30 0</trace>"##,
        r##"<traceFormat><channel name="X"/><channel name="Y"/><intermittentChannels><channel name="C" default="255"/><channel name="BW" default="4"/><channel name="A" max="255" default="128"/></intermittentChannels></traceFormat><trace>0 0,10 10,20 0 #FF0000 8</trace>"##,
        r##"<definitions><trace xml:id="a">0 0,10 10,20 0,30 10</trace></definitions><traceView traceDataRef="#a" from="2" to="3"/>"##,
        r##"<brush><brushProperty name="color" value="#ffff00"/><brushProperty name="rasterOp" value="maskPen"/></brush><trace>0 0,10 10</trace>"##,
        r##"<definitions><brush xml:id="square"><brushProperty name="tip" value="rectangle"/></brush></definitions><trace xml:id="a" continuation="begin">0 0,10 0</trace><trace brushRef="#square" continuation="end" priorRef="#a">10 0,20 0</trace>"##,
        r##"<context><canvasTransform><mapping type="unknown"/><mapping type="affine"><affine>2 0 0,0 3 0</affine></mapping></canvasTransform></context><trace>0 0,10 10</trace>"##,
        r##"<definitions><traceGroup xml:id="g"><trace xml:id="a" continuation="begin">0 0,10 10</trace><trace continuation="end" priorRef="#a">20 10,30 0</trace></traceGroup></definitions><traceView traceDataRef="#g" from="1:2" to="2:1"/>"##,
        r#"<context><canvasTransform><mapping type="unknown"/><mapping type="product"><mapping type="mathml" xmlns:m="http://www.w3.org/1998/Math/MathML"><bind source="X" variable="x"/><bind target="X"/><m:math><m:apply><m:exp/><m:ci>x</m:ci></m:apply></m:math></mapping><mapping type="affine"><bind source="Y" column="1"/><bind target="Y" column="1"/><affine>1 0</affine></mapping></mapping></canvasTransform></context><trace>1 0,2.718281828459045 1</trace>"#,
        r#"<context><canvasTransform><mapping type="unknown"/><mapping type="product"><mapping type="table"><bind source="X" column="1"/><bind target="X" column="2"/><table interpolation="cubic" apply="relative">0 0,1 2,2 8,3 18</table></mapping><mapping type="affine"><bind source="Y" column="1"/><bind target="Y" column="1"/><affine>1 0</affine></mapping></mapping></canvasTransform></context><trace>0 0,2 1,21 3</trace>"#,
    ];
    for body in variants {
        let ink = format!(r#"<ink xmlns="http://www.w3.org/2003/InkML">{body}</ink>"#);
        for (bytes, grouped) in frames(true) {
            let doc = wordcraft_docx::read(&replace_ink(&bytes, &ink)).unwrap();
            let expected = ink_graphic(&doc, grouped).items.clone();
            assert!(!expected.is_empty() && expected.iter().all(|i| !matches!(i, GraphicItem::Image { .. })));
            let mut doc: Document = serde_json::from_slice(&serde_json::to_vec(&doc).unwrap()).unwrap();
            for _ in 0..2 {
                let saved = wordcraft_docx::write(&doc).unwrap();
                let files = unzip(&saved);
                assert!(files.values().any(|b| b == ink.as_bytes()));
                assert!(files.values().any(|b| b == b"original preview bytes"));
                doc = wordcraft_docx::read(&saved).unwrap();
                assert_eq!(ink_graphic(&doc, grouped).items, expected);
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
    let bytes = docx(
        r#"<w:p><w:r><w:contentPart r:id="rInk"/></w:r></w:p>"#,
        &[("rInk", "customXml", "ink/ink1.xml")],
        &[("[Content_Types].xml", INK_TYPES), ("word/ink/ink1.xml", "malformed")],
    );
    let mut doc = wordcraft_docx::read(&bytes).unwrap();
    for _ in 0..2 {
        let objects = graphics(&doc);
        let [InlineObject::Graphic { w, h, graphic, .. }] = objects.as_slice() else { panic!("direct native ink missing") };
        assert!(*w > 0.0 && *h > 0.0, "direct placeholder needs a visible frame");
        assert_eq!((*w, *h), (graphic.w, graphic.h));
        assert!(matches!(graphic.items.as_slice(), [GraphicItem::Image { rect, media }] if media.is_empty() && *rect == [0.0, 0.0, *w, *h]));
        let saved = wordcraft_docx::write(&doc).unwrap();
        assert!(unzip(&saved).values().any(|bytes| bytes == b"malformed"));
        doc = wordcraft_docx::read(&saved).unwrap();
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
fn native_ink_inner_placement_and_rotation_keep_vectors() {
    for (from, to, frame) in [
        ("<q:xfrm>", r#"<q:xfrm rot="2700000">"#, None),
        (r#"<a:off x="0" y="0"/>"#, r#"<a:off x="127000" y="254000"/>"#, Some([10.0, 20.0, 144.0, 72.0])),
        (r#"<a:ext cx="1828800" cy="914400"/>"#, r#"<a:ext cx="914400" cy="457200"/>"#, Some([0.0, 0.0, 72.0, 36.0])),
        (r#"<a:off x="0" y="0"/>"#, r#"<a:off x="-127000" y="-254000"/>"#, Some([-10.0, -20.0, 144.0, 72.0])),
        (r#"<a:ext cx="1828800" cy="914400"/>"#, "", None),
    ] {
        let mut files = unzip(&ink_docx(true, false));
        let body = text(&files, "word/document.xml").replace(from, to);
        files.insert("word/document.xml".into(), body.into_bytes());
        let entries: Vec<_> = files.iter().map(|(path, bytes)| (path.as_str(), bytes.as_slice())).collect();
        let mut doc = wordcraft_docx::read(&zip(&entries)).unwrap();
        let expected = ink_graphic(&doc, false).items.clone();
        doc = serde_json::from_slice(&serde_json::to_vec(&doc).unwrap()).unwrap();
        for _ in 0..3 {
            let graphic = ink_graphic(&doc, false);
            assert!(matches!(graphic.items.first(), Some(GraphicItem::Path { .. })));
            assert_eq!(graphic.ink_frame, frame);
            assert_eq!(graphic.items, expected);
            assert_eq!((graphic.w, graphic.h), (144.0, 72.0));
            let saved = wordcraft_docx::write(&doc).unwrap();
            check_ink(&unzip(&saved));
            doc = wordcraft_docx::read(&saved).unwrap();
        }
    }
}

/// Repeated references charge both first-time decoding and copied vector geometry to the work cap.
#[test]
fn native_ink_decoding_and_copying_share_the_document_ink_budget() {
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

#[test]
/// Repeated direct placements share the file budget and retain sized placeholders when exhausted.
fn direct_native_ink_copy_budget_keeps_sized_placeholders() {
    let ink = format!(r#"<ink xmlns="http://www.w3.org/2003/InkML"><trace>{}</trace></ink>"#, "0 0,1 1,".repeat(30_000));
    let body = r#"<w:p><w:r><w:contentPart r:id="rInk"/></w:r></w:p>"#.repeat(65);
    let bytes = docx(&body, &[("rInk", "customXml", "ink/ink1.xml")], &[("[Content_Types].xml", INK_TYPES), ("word/ink/ink1.xml", &ink)]);
    let doc = wordcraft_docx::read(&bytes).unwrap();
    let objects = graphics(&doc);
    assert_eq!(objects.len(), 65);
    let mut vectors = 0;
    let mut placeholders = 0;
    for object in objects {
        let InlineObject::Graphic { w, h, graphic, .. } = object else { panic!("direct native ink missing") };
        assert!(graphic.source.is_some());
        match graphic.items.as_slice() {
            [GraphicItem::Image { rect, media }] => {
                assert!(w > 0.0 && h > 0.0 && media.is_empty());
                assert_eq!(*rect, [0.0, 0.0, w, h]);
                placeholders += 1;
            }
            [GraphicItem::Path { .. }] => vectors += 1,
            items => panic!("unexpected direct ink items: {items:?}"),
        }
    }
    assert!(vectors > 0 && placeholders > 0, "copy exhaustion must not hide later native ink");
}
