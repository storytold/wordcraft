//! A VML rectangle or text box filled with a picture (`v:fill type="frame"`) is drawn with the
//! picture where Word draws it, its fill colour under the picture and its outline and text over
//! it, and its text is edited by clicking into it. Word's numbers come from PDFs Word for Mac
//! exported from synthetic documents like these (A4, 72 pt margins, exact 17 pt lines, a
//! 200 × 100 px picture).

use std::io::Write;

use wordcraft_doc::fill::Fill;
use wordcraft_doc::para::ShapeKind;
use wordcraft_doc::props::Rgb;
use wordcraft_doc::{InlineObject, StoryRef};
use wordcraft_engine::Session;
use wordcraft_engine::layout::Placed;
use wordcraft_geom::Rect;

const NS: &str = r#"xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships" xmlns:v="urn:schemas-microsoft-com:vml" xmlns:o="urn:schemas-microsoft-com:office:office" xmlns:wp="http://schemas.openxmlformats.org/drawingml/2006/wordprocessingDrawing" xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" xmlns:pic="http://schemas.openxmlformats.org/drawingml/2006/picture" xmlns:wpg="http://schemas.microsoft.com/office/word/2010/wordprocessingGroup" xmlns:wps="http://schemas.microsoft.com/office/word/2010/wordprocessingShape""#;
const PAGE: &str = "mso-position-horizontal-relative:page;mso-position-vertical-relative:page";
const REL: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships";
const CYAN: Rgb = Rgb(0, 0xFF, 0xFF);

/// An opaque fill in `color`.
fn solid(color: Rgb) -> Option<Fill> {
    Some(Fill::solid(color))
}
const TEXT_BOX_TYPE: &str = r#"<v:shapetype id="_x0000_t202" coordsize="21600,21600" o:spt="202" path="m,l,21600r21600,l21600,xe"/>"#;

/// A PNG's signature and header (colour type `colour`: 2 opaque, 6 with alpha) and an empty
/// image data chunk: enough to tell whether it may be transparent.
fn png(colour: u8) -> Vec<u8> {
    let mut b = b"\x89PNG\r\n\x1a\n".to_vec();
    b.extend_from_slice(&13u32.to_be_bytes());
    b.extend_from_slice(b"IHDR");
    b.extend_from_slice(&200u32.to_be_bytes());
    b.extend_from_slice(&100u32.to_be_bytes());
    b.extend_from_slice(&[8, colour, 0, 0, 0, 0, 0, 0, 0]);
    b.extend_from_slice(&0u32.to_be_bytes());
    b.extend_from_slice(b"IDATxxxx");
    b
}

/// A document with `body`, and `header` as its header when given; `img1` is `picture`.
fn package(body: &str, header: Option<&str>, picture: &[u8]) -> Vec<u8> {
    let head_ref = if header.is_some() { r#"<w:headerReference w:type="default" r:id="rIdH"/>"# } else { "" };
    let doc = format!(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><w:document {NS}><w:body>{body}<w:sectPr>{head_ref}<w:pgSz w:w="11906" w:h="16838"/><w:pgMar w:top="1440" w:right="1440" w:bottom="1440" w:left="1440" w:header="720" w:footer="720" w:gutter="0"/></w:sectPr></w:body></w:document>"#
    );
    let root = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="{REL}/officeDocument" Target="word/document.xml"/></Relationships>"#
    );
    let image = format!(r#"<Relationship Id="img1" Type="{REL}/image" Target="media/img1.png"/>"#);
    let rels = |more: &str| {
        format!(
            r#"<?xml version="1.0" encoding="UTF-8"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">{image}{more}</Relationships>"#
        )
    };
    let head_rel = format!(r#"<Relationship Id="rIdH" Type="{REL}/header" Target="header1.xml"/>"#);
    let doc_rels = rels(if header.is_some() { &head_rel } else { "" });
    let head = header.map(|h| format!(r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><w:hdr {NS}>{h}</w:hdr>"#));
    let head_rels = rels("");
    let mut parts: Vec<(&str, &[u8])> = vec![
        ("_rels/.rels", root.as_bytes()),
        ("word/document.xml", doc.as_bytes()),
        ("word/_rels/document.xml.rels", doc_rels.as_bytes()),
        ("word/media/img1.png", picture),
    ];
    if let Some(h) = &head {
        parts.push(("word/header1.xml", h.as_bytes()));
        parts.push(("word/_rels/header1.xml.rels", head_rels.as_bytes()));
    }
    let mut zw = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    for (name, bytes) in parts {
        zw.start_file(name, zip::write::SimpleFileOptions::default()).unwrap();
        zw.write_all(bytes).unwrap();
    }
    zw.finish().unwrap().into_inner()
}

fn open_with(body: &str, header: Option<&str>, picture: &[u8]) -> Session {
    Session::new(wordcraft_engine::io::open_bytes("d.docx", &package(body, header, picture)).unwrap())
}

fn open(body: &str) -> Session {
    open_with(body, None, &png(2))
}

/// The session saved as DOCX and opened again.
fn reopened(s: &Session) -> Session {
    let bytes = wordcraft_engine::io::save_bytes("d.docx", &s.doc).unwrap();
    Session::new(wordcraft_engine::io::open_bytes("d.docx", &bytes).unwrap())
}

fn para(text: &str, runs: &str) -> String {
    format!(r#"<w:p><w:pPr><w:spacing w:before="0" w:after="0" w:line="340" w:lineRule="exact"/></w:pPr>{runs}<w:r><w:t>{text}</w:t></w:r></w:p>"#)
}

fn rect(style: &str, attrs: &str, fill: &str) -> String {
    format!(r#"<w:r><w:pict><v:rect style="position:absolute;{style};z-index:1" {attrs}>{fill}</v:rect></w:pict></w:r>"#)
}

fn text_box(style: &str, attrs: &str, text: &str) -> String {
    format!(
        r##"<w:r><w:pict>{TEXT_BOX_TYPE}<v:shape type="#_x0000_t202" style="position:absolute;{style};z-index:1;{PAGE}" {attrs}>{FRAME}<v:textbox><w:txbxContent><w:p><w:r><w:t>{text}</w:t></w:r></w:p></w:txbxContent></v:textbox></v:shape></w:pict></w:r>"##
    )
}

/// What is drawn on page 1, in drawing order: pictures, shapes (fill and outline) and the first
/// line of each text, as rects.
#[derive(Debug, PartialEq)]
enum Drawn {
    Picture(Rect),
    Shape(Rect, Option<Fill>, Option<Rgb>, f32),
    Text(f32, f32),
}

fn drawn(s: &mut Session) -> Vec<Drawn> {
    let l = s.layout();
    l.pages[0]
        .items
        .iter()
        .filter_map(|i| match i {
            Placed::Image { rect, media, spin, .. } if media == "img1.png" => {
                assert!(spin.is_identity(), "the picture is upright: {spin:?}");
                Some(Drawn::Picture(*rect))
            }
            Placed::Shape { rect, kind: ShapeKind::Rectangle | ShapeKind::TextBox, fill, stroke, stroke_width, .. } => {
                Some(Drawn::Shape(*rect, fill.clone(), *stroke, if stroke.is_some() { *stroke_width } else { 0.0 }))
            }
            Placed::Lines { x, y, story: StoryRef::Part(_), .. } => Some(Drawn::Text(*x, *y)),
            _ => None,
        })
        .collect()
}

fn pictures(d: &[Drawn]) -> Vec<Rect> {
    d.iter().filter_map(|d| if let Drawn::Picture(r) = d { Some(*r) } else { None }).collect()
}

fn outlines(d: &[Drawn]) -> Vec<(Rect, Rgb, f32)> {
    d.iter().filter_map(|d| if let Drawn::Shape(r, None, Some(c), w) = d { Some((*r, *c, *w)) } else { None }).collect()
}

fn near(a: Rect, b: [f32; 4], tol: f32) -> bool {
    [a.x - b[0], a.y - b[1], a.w - b[2], a.h - b[3]].iter().all(|d| d.abs() <= tol)
}

const FRAME: &str = r#"<v:fill type="frame" o:title="pic" r:id="img1"/>"#;

/// Word draws the picture over the shape's box, the black 0.75 pt and red 4 pt outlines over it,
/// and nothing for the shape that isn't filled.
#[test]
fn a_rectangle_filled_with_a_picture_is_drawn_with_it_under_its_outline() {
    let at = |y: u32| format!("margin-left:100pt;margin-top:{y}pt;width:200pt;height:100pt;{PAGE}");
    let runs = [
        rect(&at(100), "", FRAME),
        rect(&at(300), r#"strokecolor="red" strokeweight="4pt""#, FRAME),
        rect(&at(500), r#"stroked="f""#, FRAME),
        rect(&at(650), r#"filled="f""#, FRAME),
    ]
    .concat();
    let mut s = open(&para("ANCHOR", &runs));
    let d = drawn(&mut s);
    let images = pictures(&d);
    assert_eq!(images.len(), 3, "{images:?}");
    for (r, y) in images.iter().zip([100.0, 300.0, 500.0]) {
        assert!(near(*r, [100.0, y, 200.0, 100.0], 0.01), "{r:?}");
    }
    let lines: Vec<_> = outlines(&d).iter().map(|(r, c, w)| (r.y, *c, *w)).collect();
    assert_eq!(lines, [(100.0, Rgb::BLACK, 0.75), (300.0, Rgb(0xFF, 0, 0), 4.0), (650.0, Rgb::BLACK, 0.75)]);
}

/// A shape like a letterhead logo, placed from the column and the line, with a white outline:
/// Word draws its picture at 340.47, 59.67, 79.5 × 70.0 (the box 340.55, 59.75, 79.35 × 69.85
/// and 0.08 pt more on each side).
#[test]
fn a_logo_placed_from_its_line_is_where_word_draws_it() {
    let style = "margin-left:268.55pt;margin-top:-12.25pt;width:79.35pt;height:69.85pt;mso-position-horizontal-relative:text;mso-position-vertical-relative:line";
    let fill = r#"<v:fill type="frame" o:title="logo" recolor="t" r:id="img1"/><o:lock v:ext="edit" aspectratio="t"/>"#;
    let mut s = open(&(para("ANCHOR", &rect(style, r#"strokecolor="white [3212]""#, fill)) + &para("END", "")));
    let d = drawn(&mut s);
    let images = pictures(&d);
    assert_eq!(images.len(), 1, "{images:?}");
    assert!(near(images[0], [340.47, 59.67, 79.5, 70.0], 0.16), "{:?}", images[0]);
    let o = outlines(&d);
    assert_eq!(o.len(), 1);
    assert_eq!(o[0].1, Rgb::WHITE);
}

/// Word draws the picture at 100, 100, 200 × 100, the outline and then the text 7.55 pt in from
/// the box's left edge (its 7.2 pt margin) over it.
#[test]
fn a_text_box_filled_with_a_picture_shows_its_text_over_it() {
    let style = "margin-left:100pt;margin-top:100pt;width:200pt;height:100pt";
    let mut s = open(&para("ANCHOR", &text_box(style, "", "TEXTINBOX")));
    let d = drawn(&mut s);
    let want = [
        Drawn::Picture(Rect::new(100.0, 100.0, 200.0, 100.0)),
        Drawn::Shape(Rect::new(100.0, 100.0, 200.0, 100.0), None, Some(Rgb::BLACK), 0.75),
        Drawn::Text(107.2, 103.6),
    ];
    assert_eq!(d, want);
}

/// Word draws the shape's fill colour under the picture: a cyan rectangle and text box whose
/// picture has a transparent left half and an opaque red right half are cyan on the left, red on
/// the right, with the text box's blue outline and text over them (Word for Mac: cyan fill,
/// picture, blue outline at 100, 300, 200 × 100, then the text). The same after a save.
#[test]
fn the_fill_colour_shows_under_a_transparent_picture() {
    let at = |y: u32| format!("margin-left:100pt;margin-top:{y}pt;width:200pt;height:100pt");
    let runs = rect(&format!("{};{PAGE}", at(100)), r##"fillcolor="#00ffff""##, FRAME)
        + &text_box(&at(300), r##"fillcolor="#00ffff" strokecolor="blue""##, "TBTEXT");
    let mut s = open_with(&para("ANCHOR", &runs), None, &png(6));
    let box_at = |y: f32| Rect::new(100.0, y, 200.0, 100.0);
    let want = [
        Drawn::Shape(box_at(100.0), solid(CYAN), None, 0.0),
        Drawn::Picture(box_at(100.0)),
        Drawn::Shape(box_at(100.0), None, Some(Rgb::BLACK), 0.75),
        Drawn::Shape(box_at(300.0), solid(CYAN), None, 0.0),
        Drawn::Picture(box_at(300.0)),
        Drawn::Shape(box_at(300.0), None, Some(Rgb(0, 0, 0xFF)), 0.75),
        Drawn::Text(107.2, 303.6),
    ];
    assert_eq!(drawn(&mut s), want);
    assert_eq!(drawn(&mut reopened(&s)), want, "saved");
}

/// The members of a `v:group` filled with a picture (Word for Mac: a 300 × 200 pt group at
/// 100, 100 with `coordsize` 600,400): the pictures at 100, 100, 200 × 100 under a red 2 pt
/// outline; at 250, 175, 50 × 150, upright, for a 150 × 50 member turned 90°; at 100, 225,
/// 80 × 50 over cyan; and at 310, 100, 90 × 100 for a text box in a nested group, over cyan,
/// under its blue outline and its text.
#[test]
fn group_members_filled_with_a_picture_are_where_word_draws_them() {
    let fill2 = r#"<v:fill type="frame" o:title="pic" r:id="img1"/>"#;
    let member =
        |el: &str, style: &str, attrs: &str, inner: &str| format!(r#"<{el} style="position:absolute;{style}" {attrs}>{fill2}{inner}</{el}>"#);
    let group = [
        format!(r#"<v:group style="position:absolute;margin-left:100pt;margin-top:100pt;width:300pt;height:200pt;z-index:1;{PAGE}" coordorigin="0,0" coordsize="600,400">"#),
        member("v:rect", "left:0;top:0;width:400;height:200", r##"fillcolor="#00ffff" strokecolor="red" strokeweight="2pt""##, ""),
        member("v:rect", "left:200;top:250;width:300;height:100;rotation:90", r#"stroked="f""#, ""),
        member("v:rect", "left:0;top:250;width:160;height:100", r##"fillcolor="#00ffff" stroked="f""##, ""),
        r#"<v:group style="position:absolute;left:420;top:0;width:180;height:200" coordorigin="0,0" coordsize="180,200">"#.to_string(),
        member(
            "v:shape",
            "left:0;top:0;width:180;height:200",
            r##"type="#_x0000_t202" fillcolor="#00ffff" strokecolor="blue""##,
            "<v:textbox><w:txbxContent><w:p><w:r><w:t>GRPTEXT</w:t></w:r></w:p></w:txbxContent></v:textbox>",
        ),
        "</v:group></v:group>".to_string(),
    ]
    .concat();
    let mut s = open_with(&para("ANCHOR", &format!("<w:r><w:pict>{TEXT_BOX_TYPE}{group}</w:pict></w:r>")), None, &png(6));
    let r = Rect::new;
    let white = solid(Rgb::WHITE);
    let want = [
        Drawn::Shape(r(100.0, 100.0, 200.0, 100.0), solid(CYAN), None, 0.0),
        Drawn::Picture(r(100.0, 100.0, 200.0, 100.0)),
        Drawn::Shape(r(100.0, 100.0, 200.0, 100.0), None, Some(Rgb(0xFF, 0, 0)), 2.0),
        Drawn::Shape(r(200.0, 225.0, 150.0, 50.0), white, None, 0.0),
        Drawn::Picture(r(250.0, 175.0, 50.0, 150.0)),
        Drawn::Shape(r(100.0, 225.0, 80.0, 50.0), solid(CYAN), None, 0.0),
        Drawn::Picture(r(100.0, 225.0, 80.0, 50.0)),
        Drawn::Shape(r(310.0, 100.0, 90.0, 100.0), solid(CYAN), None, 0.0),
        Drawn::Picture(r(310.0, 100.0, 90.0, 100.0)),
        Drawn::Shape(r(310.0, 100.0, 90.0, 100.0), None, Some(Rgb(0, 0, 0xFF)), 0.75),
        Drawn::Text(317.2, 103.6),
    ];
    assert_eq!(drawn(&mut s), want);
}

/// A text box filled with a picture is edited like any text box: a click inside it goes into its
/// text, one on its border takes the box, in the body and in a header; also over a picture that
/// may be transparent, where the box's fill colour comes first in the group.
#[test]
fn clicking_inside_a_text_box_filled_with_a_picture_edits_its_text() {
    let style = "margin-left:100pt;margin-top:100pt;width:200pt;height:100pt";
    for (picture, attrs) in [(png(2), ""), (png(6), r##"fillcolor="#00ffff""##)] {
        let shape = text_box(style, attrs, "TEXTINBOX");
        let mut s = open_with(&(para("ANCHOR", &shape) + &para("END", "")), None, &picture);
        assert_eq!(first_member(&s), if attrs.is_empty() { None } else { solid(CYAN) }, "{attrs}: the fill colour first");
        clicked_into(&mut s, false, attrs);
        let mut s = open_with(&para("BODY", ""), Some(&para("HEADER", &shape)), &picture);
        clicked_into(&mut s, true, attrs);
    }
}

/// A click inside the text box at 100, 100, 200 × 100 pt on page 1, in the body or in the header
/// `in_header`, goes into its text (`TEXTINBOX`); one on its border takes the box.
fn clicked_into(s: &mut Session, in_header: bool, what: &str) {
    let l = s.layout();
    let text = |id: u32| s.doc.plain_text(StoryRef::Part(id));
    let (id, story) = if in_header {
        let head = StoryRef::Part(l.pages[0].header_story.expect("a header"));
        let Some(id) = l.header_footer_text_box_at(0, 120.0, 110.0) else { panic!("{what}: no text box in the header") };
        (id, head)
    } else {
        let Some(StoryRef::Part(id)) = l.story_at(0, 120.0, 110.0) else { panic!("{what}: {:?}", l.story_at(0, 120.0, 110.0)) };
        (id, StoryRef::Body)
    };
    assert_eq!(text(id), "TEXTINBOX", "{what}");
    assert!(l.object_at_in(0, 120.0, 110.0, 3.0, story).is_none(), "{what}: inside: the text");
    let edge = l.object_at_in(0, 100.5, 150.0, 3.0, story).unwrap_or_else(|| panic!("{what}: on the border: the box"));
    assert_eq!((edge.rect, edge.text_box), (Rect::new(100.0, 100.0, 200.0, 100.0), Some(id)), "{what}: on the border: the box");
}

/// The fill of the first member of the first group in the body, when that member is a shape.
fn first_member(s: &Session) -> Option<Fill> {
    groups(s).into_iter().find_map(|o| match o {
        InlineObject::Group { children, .. } => children.first().and_then(|c| match &c.obj {
            InlineObject::Shape { fill, .. } => fill.clone(),
            _ => None,
        }),
        _ => None,
    })
}

/// The groups in the body's paragraphs, in order.
fn groups(s: &Session) -> Vec<&InlineObject> {
    s.doc.body.iter().filter_map(|b| b.as_para()).flat_map(|p| &p.objects).filter(|o| matches!(o, InlineObject::Group { .. })).collect()
}

/// Whether each group in the body was read from a shape filled with a picture.
fn picture_fills(s: &Session) -> Vec<bool> {
    groups(s).into_iter().map(|o| matches!(o, InlineObject::Group { picture_fill: true, .. })).collect()
}

/// A DrawingML group (`wpg:wgp`) on the page at 100, 100, 200 × 100 pt: a picture and a text box
/// over it, each filling the group.
fn drawing_group(text: &str) -> String {
    named_group(text, r#"name="Group 1""#, "")
}

/// [`drawing_group`] with `doc_pr` as its `wp:docPr` attributes and `ext` in it.
fn named_group(text: &str, doc_pr: &str, ext: &str) -> String {
    let xfrm = r#"<a:xfrm><a:off x="0" y="0"/><a:ext cx="2540000" cy="1270000"/></a:xfrm>"#;
    format!(
        r#"<w:r><w:drawing><wp:anchor behindDoc="0" distT="0" distB="0" distL="0" distR="0" simplePos="0" relativeHeight="2" locked="0" layoutInCell="1" allowOverlap="1"><wp:simplePos x="0" y="0"/><wp:positionH relativeFrom="page"><wp:posOffset>1270000</wp:posOffset></wp:positionH><wp:positionV relativeFrom="page"><wp:posOffset>1270000</wp:posOffset></wp:positionV><wp:extent cx="2540000" cy="1270000"/><wp:effectExtent l="0" t="0" r="0" b="0"/><wp:wrapNone/><wp:docPr id="1" {doc_pr}>{ext}</wp:docPr><wp:cNvGraphicFramePr/><a:graphic><a:graphicData uri="http://schemas.microsoft.com/office/word/2010/wordprocessingGroup"><wpg:wgp><wpg:cNvGrpSpPr/><wpg:grpSpPr><a:xfrm><a:off x="0" y="0"/><a:ext cx="2540000" cy="1270000"/><a:chOff x="0" y="0"/><a:chExt cx="2540000" cy="1270000"/></a:xfrm></wpg:grpSpPr><pic:pic><pic:nvPicPr><pic:cNvPr id="2" name="Picture 2"/><pic:cNvPicPr/></pic:nvPicPr><pic:blipFill><a:blip r:embed="img1"/><a:stretch><a:fillRect/></a:stretch></pic:blipFill><pic:spPr>{xfrm}<a:prstGeom prst="rect"><a:avLst/></a:prstGeom></pic:spPr></pic:pic><wps:wsp><wps:cNvPr id="3" name="Text Box 3"/><wps:cNvSpPr txBox="1"/><wps:spPr>{xfrm}<a:prstGeom prst="rect"><a:avLst/></a:prstGeom><a:noFill/><a:ln w="9525"><a:solidFill><a:srgbClr val="000000"/></a:solidFill></a:ln></wps:spPr><wps:txbx><w:txbxContent><w:p><w:r><w:t>{text}</w:t></w:r></w:p></w:txbxContent></wps:txbx><wps:bodyPr/></wps:wsp></wpg:wgp></a:graphicData></a:graphic></wp:anchor></w:drawing></w:r>"#
    )
}

/// A click inside group `s` shows at 100, 100, 200 × 100 pt on page 1 takes the whole group, as
/// one on its border does, in the body or in the header `in_header`; no text box is found there.
fn taken_whole(s: &mut Session, in_header: bool, what: &str) {
    let l = s.layout();
    let story = if in_header { StoryRef::Part(l.pages[0].header_story.expect("a header")) } else { StoryRef::Body };
    for (x, y) in [(120.0, 110.0), (200.0, 150.0), (100.5, 150.0)] {
        let hit = l.object_at_in(0, x, y, 3.0, story).unwrap_or_else(|| panic!("{what}: nothing at {x}, {y}"));
        assert_eq!((hit.rect, hit.text_box), (Rect::new(100.0, 100.0, 200.0, 100.0), None), "{what}: at {x}, {y}");
    }
    assert_eq!(l.header_footer_text_box_at(0, 120.0, 110.0), None, "{what}");
}

/// A DrawingML group isn't a text box, even when its text box fills it over a picture: a click
/// inside it takes the whole group (as on `main`), in the body and in a header. Only the group
/// read from a VML shape filled with a picture is clicked into.
#[test]
fn an_ordinary_group_whose_text_box_fills_it_is_taken_whole() {
    let group = drawing_group("GROUPTEXT");
    let mut s = open(&(para("ANCHOR", &group) + &para("END", "")));
    assert_eq!(picture_fills(&s), [false]);
    taken_whole(&mut s, false, "body");
    let mut s = open_with(&para("BODY", ""), Some(&para("HEADER", &group)), &png(2));
    taken_whole(&mut s, true, "header");
}

/// A group named and described like our mark, with an extension of another URI holding the
/// mark's element, is an ordinary group, also once saved: only the mark's URI makes one.
#[test]
fn a_group_named_like_a_picture_fill_is_taken_whole() {
    let doc_pr = r#"name="Picture fill 1" descr="urn:wordcraft:picture-fill" title="pictureFill""#;
    let ext =
        r#"<a:extLst><a:ext uri="urn:wordcraft:picture-fill:v2"><wcpf:pictureFill xmlns:wcpf="urn:wordcraft:picture-fill"/></a:ext></a:extLst>"#;
    let mut s = open(&(para("ANCHOR", &named_group("GROUPTEXT", doc_pr, ext)) + &para("END", "")));
    assert_eq!(picture_fills(&s), [false]);
    taken_whole(&mut s, false, "named");
    let mut back = reopened(&s);
    assert_eq!(picture_fills(&back), [false]);
    taken_whole(&mut back, false, "named, saved");
    for d in [&s, &back] {
        let alt = groups(d).first().and_then(|o| o.frame()).map(|(_, _, f)| (f.alt.text.clone(), f.alt.title.clone()));
        assert_eq!(alt, Some(("urn:wordcraft:picture-fill".into(), "pictureFill".into())));
    }
}

/// Saved and opened again, a text box filled with a picture is still clicked into, in the body
/// and in a header, over an opaque picture and over a transparent one: the group keeps its mark
/// (in its `wp:docPr`), and its description, title and decorative mark stay as they were. Saved
/// twice, the same.
#[test]
fn saved_a_text_box_filled_with_a_picture_is_still_clicked_into() {
    let style = "margin-left:100pt;margin-top:100pt;width:200pt;height:100pt";
    for (picture, attrs) in [(png(2), ""), (png(6), r##"fillcolor="#00ffff""##)] {
        let shape = text_box(style, attrs, "TEXTINBOX");
        let mut s = open_with(&(para("ANCHOR", &shape) + &para("END", "")), None, &picture);
        for b in &mut s.doc.body {
            if let wordcraft_doc::Block::Para(p) = std::sync::Arc::make_mut(b) {
                for o in &mut p.objects {
                    if let InlineObject::Group { float, .. } = o {
                        float.alt.text = "Firm logo".into();
                        float.alt.title = "Logo".into();
                        float.alt.decorative = !attrs.is_empty();
                    }
                }
            }
        }
        let mut back = reopened(&s);
        assert_eq!(picture_fills(&back), [true], "{attrs}");
        assert_eq!(first_member(&back), first_member(&s), "{attrs}");
        let alt = groups(&back).first().and_then(|o| o.frame()).map(|(_, _, f)| f.alt.clone());
        let want = wordcraft_doc::para::AltText { text: "Firm logo".into(), title: "Logo".into(), decorative: !attrs.is_empty() };
        assert_eq!(alt, Some(want), "{attrs}");
        clicked_into(&mut back, false, &format!("saved {attrs}"));
        let mut twice = reopened(&back);
        assert_eq!(picture_fills(&twice), [true], "{attrs}");
        clicked_into(&mut twice, false, &format!("saved twice {attrs}"));
        let s = open_with(&para("BODY", ""), Some(&para("HEADER", &shape)), &picture);
        clicked_into(&mut reopened(&s), true, &format!("saved {attrs}"));
    }
}

/// Copied and pasted, undone and redone, or kept for recovery, the group is still the text box
/// filled with a picture; an ordinary group is never written as one.
#[test]
fn a_text_box_filled_with_a_picture_stays_one_when_copied_and_undone() {
    let style = "margin-left:100pt;margin-top:100pt;width:200pt;height:100pt";
    let mut s = open(&(para("ANCHOR", &text_box(style, "", "TEXTINBOX")) + &para("END", "")));
    let run = |s: &mut Session, id: &str| s.run(id, &serde_json::json!({})).unwrap();
    run(&mut s, "select.all");
    run(&mut s, "edit.copy");
    run(&mut s, "caret.docEnd");
    run(&mut s, "edit.paste");
    assert_eq!(picture_fills(&s), [true, true], "pasted");
    run(&mut s, "edit.undo");
    assert_eq!(picture_fills(&s), [true], "undone");
    run(&mut s, "edit.redo");
    assert_eq!(picture_fills(&s), [true, true], "redone");
    let json = serde_json::to_value(&s.doc).unwrap();
    let kept: wordcraft_doc::Document = serde_json::from_value(json).unwrap();
    assert_eq!(kept, s.doc, "recovered");
    let group = open(&para("ANCHOR", &drawing_group("GROUPTEXT")));
    assert!(serde_json::to_string(&s.doc).unwrap().contains(r#""picture_fill":true"#));
    assert!(!serde_json::to_string(&group.doc).unwrap().contains("picture_fill"));
}

/// A text box turned half round keeps its picture and its text upright (Word for Mac: the
/// picture at 100, 400, 200 × 100 as drawn unturned, the text at the box's top left).
#[test]
fn a_text_box_turned_half_round_keeps_its_picture_upright() {
    let style = "margin-left:100pt;margin-top:400pt;width:200pt;height:100pt;rotation:180";
    let mut s = open(&para("ANCHOR", &text_box(style, r##"fillcolor="#00ffff""##, "QTEXT")));
    let want = [
        Drawn::Picture(Rect::new(100.0, 400.0, 200.0, 100.0)),
        Drawn::Shape(Rect::new(100.0, 400.0, 200.0, 100.0), None, Some(Rgb::BLACK), 0.75),
        Drawn::Text(107.2, 403.6),
    ];
    assert_eq!(drawn(&mut s), want);
}

/// Where a click on page 1 goes, over a grid around the box at 100, 100, 200 × 100 pt, in the
/// body or in the header `in_header`: into the text box's text, the text around it, or the box
/// on its border; and where the caret at the start of the box's text is.
fn clicks(s: &mut Session, in_header: bool) -> Vec<String> {
    let l = s.layout();
    let Some(id) = s.doc.parts.iter().find(|(_, p)| p.kind == wordcraft_doc::PartKind::TextBox).map(|(id, _)| *id) else { return Vec::new() };
    let story = if in_header { l.pages[0].header_story.map(StoryRef::Part).unwrap_or(StoryRef::Body) } else { StoryRef::Body };
    let mut out = Vec::new();
    for y in (40..=260).step_by(10) {
        for x in (90..=310).step_by(10) {
            let (x, y) = (x as f32, y as f32);
            let text = if in_header { l.header_footer_text_box_at(0, x, y).map(StoryRef::Part) } else { l.story_at(0, x, y) };
            let text = if text == Some(StoryRef::Part(id)) { "box text".to_string() } else { format!("{text:?}") };
            let grabbed = l.object_at_in(0, x, y, 3.0, story).map(|o| (o.rect, o.spin, o.text_box == Some(id)));
            out.push(format!("{x}, {y}: {text}, {grabbed:?}"));
        }
    }
    let start = wordcraft_doc::Pos { story: StoryRef::Part(id), path: wordcraft_doc::Path::top(0), off: 0 };
    out.push(format!("caret {:?}", l.caret(&start).map(|c| (c.x, c.top, c.height))));
    out
}

/// Turned a quarter or three quarters round, a text box filled with a picture takes clicks, in
/// the body and in a header, where the same text box without the picture does (as on `main`,
/// which reads it so): its text where the text is laid out, upright in the unturned frame (its
/// first letters at 120, 110 lie outside the turned frame), and the box on its turned border;
/// the caret is where that text box has it.
#[test]
fn a_turned_text_box_filled_with_a_picture_takes_clicks_as_one_without_it() {
    for deg in [90, 270] {
        let style = format!("margin-left:100pt;margin-top:100pt;width:200pt;height:100pt;rotation:{deg}");
        let filled = text_box(&style, r##"fillcolor="#00ffff""##, "TEXTINBOX");
        let plain = filled.replace(FRAME, "");
        for in_header in [false, true] {
            let doc = |runs: &str| {
                if in_header {
                    open_with(&para("BODY", ""), Some(&para("HEADER", runs)), &png(6))
                } else {
                    open_with(&(para("ANCHOR", runs) + &para("END", "")), None, &png(6))
                }
            };
            let (mut with, mut without) = (doc(&filled), doc(&plain));
            if !in_header {
                assert_eq!(picture_fills(&with), [true], "{deg}°");
                assert!(groups(&without).is_empty(), "{deg}°");
            }
            let want = clicks(&mut without, in_header);
            assert!(want.iter().any(|c| c.starts_with("120, 110: box text")), "{deg}°, header {in_header}: the first letters: {want:#?}");
            assert_eq!(clicks(&mut with, in_header), want, "{deg}°, header {in_header}");
        }
    }
}

/// The text box filled with a picture (cyan under a picture that may be transparent) at 100, 100
/// and a red rectangle at 100, 300, both 200 × 100 pt on the page, anchored in one paragraph.
fn text_box_and_rectangle() -> Session {
    let style = "margin-left:100pt;margin-top:100pt;width:200pt;height:100pt";
    let red = rect(&format!("margin-left:100pt;margin-top:300pt;width:200pt;height:100pt;{PAGE}"), r##"fillcolor="#ff0000""##, "");
    let runs = text_box(style, r##"fillcolor="#00ffff""##, "TEXTINBOX") + &red;
    open_with(&(para("ANCHOR", &runs) + &para("END", "")), None, &png(6))
}

/// Run command `id` with `params`.
fn run(s: &mut Session, id: &str, params: serde_json::Value) -> Result<serde_json::Value, wordcraft_engine::CmdError> {
    s.run(id, &params)
}

/// The objects at these offsets of the first paragraph, selected together.
fn select_objects(s: &mut Session, offs: &[usize]) {
    let objects: Vec<_> = offs.iter().map(|o| serde_json::to_value(wordcraft_doc::Pos::body(0, *o)).unwrap()).collect();
    run(s, "select.addObject", serde_json::json!({ "objects": objects })).unwrap();
}

/// What page 1 shows, in no particular order (a group draws its text boxes' text after its
/// members).
fn shown(s: &mut Session) -> Vec<String> {
    let mut d: Vec<String> = drawn(s).iter().map(|d| format!("{d:?}")).collect();
    d.sort();
    d
}

/// A click anywhere in the group at 100, 100, 200 × 300 pt on page 1, the former text box
/// included, takes the whole group.
fn group_taken_whole(s: &mut Session, what: &str) {
    let l = s.layout();
    for (x, y) in [(120.0, 110.0), (200.0, 150.0), (200.0, 350.0)] {
        let hit = l.object_at(0, x, y, 3.0).unwrap_or_else(|| panic!("{what}: nothing at {x}, {y}"));
        assert_eq!((hit.rect, hit.text_box), (Rect::new(100.0, 100.0, 200.0, 300.0), None), "{what}: at {x}, {y}");
    }
}

/// Grouped with another shape (Layout › Arrange › Group), a text box filled with a picture is a
/// member as any text box is on `main` (where it is read without its picture): its layers join
/// the new group, an ordinary one drawn as before, which a click anywhere in takes whole, also
/// saved and reopened. Undone, it is the text box filled with a picture again, clicked into;
/// redone, the group.
#[test]
fn grouped_with_a_shape_a_text_box_filled_with_a_picture_is_a_member() {
    let mut s = text_box_and_rectangle();
    let before = shown(&mut s);
    assert_eq!(picture_fills(&s), [true]);
    select_objects(&mut s, &[0, 3]);
    run(&mut s, "arrange.group", serde_json::json!({})).unwrap();
    assert_eq!(picture_fills(&s), [false]);
    let members = groups(&s).first().map(|o| match o {
        InlineObject::Group { children, .. } => children.len(),
        _ => 0,
    });
    assert_eq!(members, Some(4), "the fill colour, the picture, the text box and the rectangle");
    assert_eq!(shown(&mut s), before, "drawn as before");
    group_taken_whole(&mut s, "grouped");
    let mut back = reopened(&s);
    assert_eq!(picture_fills(&back), [false]);
    assert_eq!(shown(&mut back), before, "saved");
    group_taken_whole(&mut back, "saved");
    run(&mut s, "edit.undo", serde_json::json!({})).unwrap();
    assert_eq!(picture_fills(&s), [true], "undone");
    assert_eq!(shown(&mut s), before, "undone");
    clicked_into(&mut s, false, "undone");
    run(&mut s, "edit.redo", serde_json::json!({})).unwrap();
    assert_eq!(picture_fills(&s), [false], "redone");
    group_taken_whole(&mut s, "redone");
}

/// A text box filled with a picture is one shape, as on `main` and in Word: Ungroup is greyed
/// out for it and refused, so it stays whole and clicked into, also saved and reopened. (Its
/// layers apart, a click inside the text box would take the picture under it.)
#[test]
fn a_text_box_filled_with_a_picture_cant_be_ungrouped() {
    let mut s = text_box_and_rectangle();
    select_objects(&mut s, &[0]);
    assert_eq!(s.why_disabled("arrange.ungroup"), Some("a shape filled with a picture is one shape: it can't be ungrouped"));
    let err = run(&mut s, "arrange.ungroup", serde_json::json!({})).unwrap_err();
    assert!(matches!(err, wordcraft_engine::CmdError::Disabled(_)), "{err:?}");
    assert_eq!(picture_fills(&s), [true]);
    assert_eq!(s.doc.body.first().and_then(|b| b.as_para()).map(|p| p.objects.len()), Some(2));
    clicked_into(&mut s, false, "not ungrouped");
    let mut back = reopened(&s);
    select_objects(&mut back, &[0]);
    assert!(back.why_disabled("arrange.ungroup").is_some(), "saved");
    clicked_into(&mut back, false, "saved");
}

/// A group made from it with another shape is an ordinary group: Ungroup splits it into its
/// members, drawn as before (the text box's layers as three objects).
#[test]
fn a_group_made_with_a_text_box_filled_with_a_picture_ungroups_into_its_members() {
    let mut s = text_box_and_rectangle();
    let before = shown(&mut s);
    select_objects(&mut s, &[0, 3]);
    run(&mut s, "arrange.group", serde_json::json!({})).unwrap();
    assert_eq!(s.why_disabled("arrange.ungroup"), None);
    run(&mut s, "arrange.ungroup", serde_json::json!({})).unwrap();
    assert!(groups(&s).is_empty());
    assert_eq!(shown(&mut s), before, "drawn as before");
    run(&mut s, "edit.undo", serde_json::json!({})).unwrap();
    assert_eq!(picture_fills(&s), [false], "undone: the group");
}

/// A floating `v:group` of `n` small rectangles at 100, 400, 200 × 100 pt on the page, and the
/// text box filled with a picture that may be transparent (three layers) at 100, 100.
fn rectangles_and_text_box(n: usize) -> Session {
    let rects: String =
        (0..n).map(|i| format!(r#"<v:rect style="position:absolute;left:{};top:0;width:1;height:1" stroked="f"/>"#, i % 200)).collect();
    let group = format!(
        r#"<w:r><w:pict><v:group style="position:absolute;margin-left:100pt;margin-top:400pt;width:200pt;height:100pt;z-index:1;{PAGE}" coordorigin="0,0" coordsize="200,100">{rects}</v:group></w:pict></w:r>"#
    );
    let style = "margin-left:100pt;margin-top:100pt;width:200pt;height:100pt";
    let runs = group + &text_box(style, r##"fillcolor="#00ffff""##, "TEXTINBOX");
    open_with(&(para("ANCHOR", &runs) + &para("END", "")), None, &png(6))
}

/// A text box filled with a picture counts as its three layers against the
/// `MAX_GROUP_CHILDREN` members Group (Layout › Arrange › Group) makes a group of: 997
/// rectangles and the text box make one, 998 are refused and leave both as they were, the text
/// box still clicked into.
#[test]
fn group_counts_a_picture_filled_text_box_as_its_layers() {
    let limit = wordcraft_doc::para::MAX_GROUP_CHILDREN;
    let mut s = rectangles_and_text_box(limit - 3);
    select_objects(&mut s, &[0, 3]);
    run(&mut s, "arrange.group", serde_json::json!({})).unwrap();
    let members = groups(&s).first().map(|o| match o {
        InlineObject::Group { children, .. } => children.len(),
        _ => 0,
    });
    assert_eq!(members, Some(limit));
    let mut s = rectangles_and_text_box(limit - 2);
    assert_eq!(picture_fills(&s), [false, true]);
    select_objects(&mut s, &[0, 3]);
    let err = run(&mut s, "arrange.group", serde_json::json!({})).unwrap_err();
    assert!(matches!(err, wordcraft_engine::CmdError::Disabled(_)), "{err:?}");
    assert_eq!(picture_fills(&s), [false, true]);
    clicked_into(&mut s, false, "not grouped");
}

/// Selected as an object, a text box filled with a picture takes the Font commands into all its
/// text, as a text box without a picture does and as Word has it (one text box): the body text
/// around it is left as it was, and Bold reads the box's text, so pressing it again takes it off.
#[test]
fn font_commands_on_a_selected_text_box_filled_with_a_picture_format_its_text() {
    let mut s = text_box_and_rectangle();
    let story = groups(&s).first().and_then(|o| o.text_boxes().first().copied()).map(StoryRef::Part).expect("its text box");
    let runs = |s: &Session, story: StoryRef| -> Vec<wordcraft_doc::props::CharProps> {
        s.doc.para_paths(story).iter().filter_map(|p| s.doc.para(story, p)).flat_map(|p| p.run_ranges().map(|(_, c)| c.clone())).collect()
    };
    select_objects(&mut s, &[0]);
    run(&mut s, "format.font", serde_json::json!({"name": "Arial"})).unwrap();
    run(&mut s, "format.bold", serde_json::json!({})).unwrap();
    let boxed = runs(&s, story);
    assert!(!boxed.is_empty());
    assert!(boxed.iter().all(|c| c.font.as_deref() == Some("Arial") && c.bold == Some(true)), "{boxed:?}");
    let body = runs(&s, StoryRef::Body);
    assert!(body.iter().all(|c| c.font.is_none() && c.bold.is_none()), "{body:?}");
    run(&mut s, "format.bold", serde_json::json!({})).unwrap();
    assert!(runs(&s, story).iter().all(|c| c.bold == Some(false)));
    assert_eq!(picture_fills(&s), [true]);
}
