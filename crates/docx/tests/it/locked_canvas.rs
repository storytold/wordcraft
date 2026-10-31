//! A DrawingML locked canvas (`lc:lockedCanvas`, ECMA-376 Part 1 §20.3.2.1), as Word 2007 writes
//! shapes, pictures and WordArt in a drawing in compatibility mode (#5060). Fixtures are
//! hand-written from ECMA-376 Part 1 §20.1 (DrawingML main) and §20.3 (locked canvas).

use std::io::Write;

use wordcraft_doc::para::ShapeKind;
use wordcraft_doc::props::{Rgb, TextColor};
use wordcraft_doc::{Document, InlineObject, StoryRef};

const NS: &str = r#"xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships" xmlns:wp="http://schemas.openxmlformats.org/drawingml/2006/wordprocessingDrawing" xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main""#;

fn docx(body: &str) -> Vec<u8> {
    let doc = format!(r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><w:document {NS}><w:body>{body}</w:body></w:document>"#);
    let types =
        r#"<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="png" ContentType="image/png"/></Types>"#;
    let root = r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="word/document.xml"/></Relationships>"#;
    let rels = r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId8" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/image" Target="media/image1.png"/></Relationships>"#;
    let entries: [(&str, &[u8]); 5] = [
        ("[Content_Types].xml", types.as_bytes()),
        ("_rels/.rels", root.as_bytes()),
        ("word/document.xml", doc.as_bytes()),
        ("word/_rels/document.xml.rels", rels.as_bytes()),
        ("word/media/image1.png", "\u{89}PNG stand-in".as_bytes()),
    ];
    let mut zw = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    for (name, bytes) in entries {
        zw.start_file(name, zip::write::SimpleFileOptions::default()).unwrap();
        zw.write_all(bytes).unwrap();
    }
    zw.finish().unwrap().into_inner()
}

/// A floating drawing, 200 × 100 pt, holding `canvas` (the locked canvas's members) in a canvas
/// whose members' space starts at 10 pt, 10 pt.
fn drawing(canvas: &str) -> String {
    format!(
        r#"<w:p><w:r><w:drawing><wp:anchor behindDoc="0" distT="0" distB="0" distL="0" distR="0" simplePos="0" relativeHeight="1" locked="0" layoutInCell="1" allowOverlap="1"><wp:simplePos x="0" y="0"/><wp:positionH relativeFrom="column"><wp:posOffset>0</wp:posOffset></wp:positionH><wp:positionV relativeFrom="paragraph"><wp:posOffset>0</wp:posOffset></wp:positionV><wp:extent cx="2540000" cy="1270000"/><wp:wrapNone/><wp:docPr id="1" name="Object 1"/><a:graphic><a:graphicData uri="http://schemas.openxmlformats.org/drawingml/2006/lockedCanvas"><lc:lockedCanvas xmlns:lc="http://schemas.openxmlformats.org/drawingml/2006/lockedCanvas"><a:nvGrpSpPr><a:cNvPr id="0" name=""/><a:cNvGrpSpPr/></a:nvGrpSpPr><a:grpSpPr><a:xfrm><a:off x="0" y="0"/><a:ext cx="2540000" cy="1270000"/><a:chOff x="127000" y="127000"/><a:chExt cx="2540000" cy="1270000"/></a:xfrm></a:grpSpPr>{canvas}</lc:lockedCanvas></a:graphicData></a:graphic></wp:anchor></w:drawing></w:r></w:p>"#
    )
}

const OVAL: &str = r#"<a:sp><a:nvSpPr><a:cNvPr id="2" name="Oval 2"/><a:cNvSpPr/></a:nvSpPr><a:spPr><a:xfrm><a:off x="127000" y="127000"/><a:ext cx="1270000" cy="635000"/></a:xfrm><a:prstGeom prst="ellipse"><a:avLst/></a:prstGeom><a:solidFill><a:srgbClr val="FFFF66"/></a:solidFill><a:ln w="12700"><a:solidFill><a:srgbClr val="000000"/></a:solidFill></a:ln></a:spPr></a:sp>"#;

/// WordArt in a nested group: bold, all-caps red text with a green outline and a reflection,
/// 32 pt, centred, bent by a chevron warp; a second paragraph takes the list style's size, and an
/// empty third one its mark's.
const WORD_ART: &str = r#"<a:grpSp><a:nvGrpSpPr><a:cNvPr id="3" name="Group 3"/><a:cNvGrpSpPr/></a:nvGrpSpPr><a:grpSpPr><a:xfrm><a:off x="1397000" y="762000"/><a:ext cx="1270000" cy="635000"/><a:chOff x="0" y="0"/><a:chExt cx="2540000" cy="1270000"/></a:xfrm></a:grpSpPr><a:sp><a:nvSpPr><a:cNvPr id="4" name="Rectangle 4"/><a:cNvSpPr/></a:nvSpPr><a:spPr><a:xfrm><a:off x="254000" y="254000"/><a:ext cx="508000" cy="254000"/></a:xfrm><a:prstGeom prst="rect"><a:avLst/></a:prstGeom><a:noFill/></a:spPr><a:txSp><a:txBody><a:bodyPr wrap="square"><a:prstTxWarp prst="textChevron"><a:avLst/></a:prstTxWarp><a:spAutoFit/></a:bodyPr><a:lstStyle><a:lvl1pPr><a:defRPr sz="1400"/></a:lvl1pPr></a:lstStyle><a:p><a:pPr algn="ctr"/><a:r><a:rPr lang="en-US" sz="3200" b="1" cap="all"><a:ln w="9525"><a:solidFill><a:srgbClr val="00FF00"/></a:solidFill></a:ln><a:solidFill><a:srgbClr val="FF0000"/></a:solidFill><a:effectLst><a:reflection blurRad="12700" stA="28000" endPos="45000" dist="1000" dir="5400000"/></a:effectLst><a:latin typeface="Corsiva Stand-in"/></a:rPr><a:t>Young</a:t></a:r></a:p><a:p><a:r><a:rPr lang="en-US"/><a:t>keepers</a:t></a:r></a:p><a:p><a:endParaRPr lang="en-US" sz="1000"/></a:p></a:txBody><a:useSpRect/></a:txSp></a:sp></a:grpSp>"#;

const PICTURE: &str = r#"<a:pic><a:nvPicPr><a:cNvPr id="5" name="Picture 5" descr="A logo"/><a:cNvPicPr/></a:nvPicPr><a:blipFill><a:blip r:embed="rId8"/><a:stretch><a:fillRect/></a:stretch></a:blipFill><a:spPr><a:xfrm><a:off x="127000" y="762000"/><a:ext cx="635000" cy="635000"/></a:xfrm><a:prstGeom prst="rect"><a:avLst/></a:prstGeom></a:spPr></a:pic>"#;

/// A line glued to the oval by the canvas's own id, which isn't a document shape's.
const LINE: &str = r#"<a:cxnSp><a:nvCxnSpPr><a:cNvPr id="6" name="Line 6"/><a:cNvCxnSpPr><a:stCxn id="2" idx="0"/></a:cNvCxnSpPr></a:nvCxnSpPr><a:spPr><a:xfrm><a:off x="127000" y="1143000"/><a:ext cx="2540000" cy="0"/></a:xfrm><a:prstGeom prst="line"><a:avLst/></a:prstGeom><a:ln w="12700"><a:solidFill><a:srgbClr val="0000FF"/></a:solidFill></a:ln></a:spPr></a:cxnSp>"#;

fn group(d: &Document) -> (f32, f32, Vec<wordcraft_doc::para::GroupChild>) {
    let p = d.body.iter().find_map(|b| b.as_para()).unwrap();
    match p.objects.first() {
        Some(InlineObject::Group { w, h, children, canvas: None, .. }) => (*w, *h, children.clone()),
        o => panic!("{o:?}"),
    }
}

/// #5060: the locked canvas is a group of its shapes, text, picture and line, each where the
/// canvas (and the group inside it) puts it, and nothing is lost on save.
#[test]
fn a_locked_canvas_reads_as_a_group_of_its_members() {
    let d = wordcraft_docx::read(&docx(&drawing(&[OVAL, WORD_ART, PICTURE, LINE].concat()))).unwrap();
    for d in [&d, &wordcraft_docx::read(&wordcraft_docx::write(&d).unwrap()).unwrap()] {
        let (w, h, children) = group(d);
        assert_eq!((w, h), (200.0, 100.0));
        let at: Vec<(f32, f32)> = children.iter().map(|c| (c.x, c.y)).collect();
        assert_eq!(at, [(0.0, 0.0), (110.0, 60.0), (0.0, 50.0), (0.0, 80.0)]);
        let [oval, art, picture, line] = &children[..] else { panic!("{children:?}") };
        let InlineObject::Shape { kind: ShapeKind::Ellipse, w: 100.0, h: 50.0, fill: Some(fill), stroke: Some(stroke), .. } = &oval.obj else {
            panic!("{:?}", oval.obj)
        };
        assert_eq!((fill.main_color(), *stroke), (Rgb(0xFF, 0xFF, 0x66), Rgb(0, 0, 0)));
        let InlineObject::Shape { kind: ShapeKind::TextBox, w: 20.0, h: 10.0, story: Some(id), extra, .. } = &art.obj else {
            panic!("{:?}", art.obj)
        };
        assert_eq!(extra.warp.as_ref().map(|w| w.preset.as_str()), Some("textChevron"));
        assert_eq!(d.plain_text(StoryRef::Part(*id)).trim_end(), "Young\nkeepers");
        let blocks = &d.parts.get(id).unwrap().blocks;
        let first = blocks[0].as_para().unwrap();
        assert_eq!(first.props.align, Some(wordcraft_doc::props::Align::Center));
        assert_eq!((first.props.space_before, first.props.space_after), (Some(0.0), Some(0.0)), "DrawingML text has no paragraph spacing");
        let run = &first.runs[0].props;
        assert_eq!((run.size, run.bold, run.caps, run.color), (Some(32.0), Some(true), Some(true), Some(TextColor::Rgb(Rgb(0xFF, 0, 0)))));
        assert_eq!(run.font.as_deref(), Some("Corsiva Stand-in"));
        let fx = run.text_effects.as_deref().unwrap();
        let outline = fx.outline.unwrap();
        assert_eq!((outline.color, outline.width), (Some(Rgb(0, 0xFF, 0)), 0.75));
        assert!(fx.reflection.is_some(), "{fx:?}");
        assert_eq!(blocks[1].as_para().unwrap().runs[0].props.size, Some(14.0), "the list style's size");
        assert_eq!(blocks[2].as_para().unwrap().mark.size, Some(10.0), "an empty paragraph's mark");
        let InlineObject::Image { w: 50.0, h: 50.0, alt, .. } = &picture.obj else { panic!("{:?}", picture.obj) };
        assert_eq!(alt, "A logo");
        let InlineObject::Shape { kind: ShapeKind::Line, stroke: Some(blue), extra, .. } = &line.obj else { panic!("{:?}", line.obj) };
        assert_eq!((*blue, extra.start), (Rgb(0, 0, 0xFF), None), "no glue to a canvas id");
    }
}

/// A text shape as a member of the canvas (`a:txSp`, text with no shape of its own) is a text box
/// at its own rectangle. Runs take their paragraph's own defaults (`a:pPr/a:defRPr`) before the
/// list style's for the paragraph's level.
#[test]
fn a_locked_canvas_text_shape_and_paragraph_defaults() {
    let text = r#"<a:txSp><a:txBody><a:bodyPr/><a:lstStyle><a:lvl1pPr><a:defRPr sz="1400" i="1"/></a:lvl1pPr><a:lvl2pPr><a:defRPr sz="2000"/></a:lvl2pPr></a:lstStyle><a:p><a:pPr><a:defRPr sz="3200"/></a:pPr><a:r><a:rPr lang="en-US"/><a:t>Own</a:t></a:r></a:p><a:p><a:pPr lvl="1"/><a:r><a:rPr lang="en-US"/><a:t>Level</a:t></a:r></a:p><a:p><a:r><a:rPr lang="en-US"/><a:t>First</a:t></a:r></a:p></a:txBody><a:useSpRect/><a:xfrm><a:off x="635000" y="381000"/><a:ext cx="1270000" cy="508000"/></a:xfrm></a:txSp>"#;
    let d = wordcraft_docx::read(&docx(&drawing(text))).unwrap();
    for d in [&d, &wordcraft_docx::read(&wordcraft_docx::write(&d).unwrap()).unwrap()] {
        let (_, _, children) = group(d);
        let [shape] = &children[..] else { panic!("{children:?}") };
        assert_eq!((shape.x, shape.y), (40.0, 20.0));
        let InlineObject::Shape { kind: ShapeKind::TextBox, w: 100.0, h: 40.0, fill: None, stroke: None, story: Some(id), .. } = &shape.obj else {
            panic!("{:?}", shape.obj)
        };
        assert_eq!(d.plain_text(StoryRef::Part(*id)).trim_end(), "Own\nLevel\nFirst");
        let blocks = &d.parts.get(id).unwrap().blocks;
        let run = |i: usize| blocks[i].as_para().unwrap().runs[0].props.clone();
        assert_eq!((run(0).size, run(0).italic), (Some(32.0), Some(true)), "the paragraph's own size, the level's italic");
        assert_eq!((run(1).size, run(1).italic), (Some(20.0), None), "level 2's");
        assert_eq!((run(2).size, run(2).italic), (Some(14.0), Some(true)), "level 1's");
    }
}

/// Before #5060 only the canvas's first picture showed: the shapes and their text were dropped.
#[test]
fn a_locked_canvas_is_not_read_as_its_picture_alone() {
    let d = wordcraft_docx::read(&docx(&drawing(&[OVAL, PICTURE].concat()))).unwrap();
    let (_, _, children) = group(&d);
    assert_eq!(children.len(), 2);
}

/// Hostile canvases never panic: no members, groups nested past the limit, text with no runs,
/// huge and broken sizes.
#[test]
fn hostile_locked_canvases_never_panic() {
    let deep = format!("{}{}{}", r#"<a:grpSp><a:grpSpPr/>"#.repeat(40), OVAL, "</a:grpSp>".repeat(40));
    let odd = r#"<a:sp><a:spPr/><a:txSp><a:txBody><a:p/><a:p><a:r><a:rPr sz="99999999999" baseline="x"/></a:r><a:br/><a:fld><a:t>f</a:t></a:fld></a:p></a:txBody></a:txSp></a:sp><a:pic/><a:cxnSp/><a:graphicFrame/>"#;
    for canvas in ["", deep.as_str(), odd] {
        let d = wordcraft_docx::read(&docx(&drawing(canvas))).unwrap();
        let _ = wordcraft_docx::write(&d).unwrap();
    }
}
