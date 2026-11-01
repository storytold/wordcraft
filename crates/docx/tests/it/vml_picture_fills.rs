//! VML rectangles and text boxes filled with a picture (`v:fill type="frame"`, ECMA-376 Part 4,
//! VML `fill`), read as Word draws them: the shape's fill colour under the picture when the
//! picture may let it through, the picture stretched over the shape, then its outline and text.
//! Fixtures are hand-written; the pictures are PNG and JPEG headers built here.

use std::io::Write;

use wordcraft_doc::fill::Fill;
use wordcraft_doc::para::{Anchor, ShapeKind, Wrap};
use wordcraft_doc::props::Rgb;
use wordcraft_doc::{Document, InlineObject, Paragraph};

const NS: &str = r#"xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships" xmlns:v="urn:schemas-microsoft-com:vml" xmlns:o="urn:schemas-microsoft-com:office:office""#;
const PAGE: &str = "position:absolute;margin-left:100pt;margin-top:100pt;width:200pt;height:100pt;z-index:1;mso-position-horizontal-relative:page;mso-position-vertical-relative:page";
const FRAME: &str = r#"<v:fill type="frame" o:title="pic" r:id="rIdImg"/>"#;
const TEXT_BOX_TYPE: &str = r#"<v:shapetype id="_x0000_t202" coordsize="21600,21600" o:spt="202" path="m,l,21600r21600,l21600,xe"/>"#;
const CYAN: Rgb = Rgb(0, 0xFF, 0xFF);

/// An opaque fill in `color`.
fn solid(color: Rgb) -> Option<Fill> {
    Some(Fill::solid(color))
}

/// A PNG's signature, its header with colour type `colour` and the chunks named `chunks`, empty:
/// enough for its kind of colour to be told.
fn png(colour: u8, chunks: &[&[u8; 4]]) -> Vec<u8> {
    let mut b = b"\x89PNG\r\n\x1a\n".to_vec();
    b.extend_from_slice(&13u32.to_be_bytes());
    b.extend_from_slice(b"IHDR");
    b.extend_from_slice(&200u32.to_be_bytes());
    b.extend_from_slice(&100u32.to_be_bytes());
    b.extend_from_slice(&[8, colour, 0, 0, 0, 0, 0, 0, 0]);
    for name in chunks {
        b.extend_from_slice(&0u32.to_be_bytes());
        b.extend_from_slice(*name);
        b.extend_from_slice(&[0; 4]);
    }
    b
}

/// An opaque PNG (true colour, no transparent colour).
fn opaque() -> Vec<u8> {
    png(2, &[b"IDAT"])
}

/// A PNG with an alpha channel.
fn see_through() -> Vec<u8> {
    png(6, &[b"IDAT"])
}

fn read(body: &str, picture: &[u8]) -> Document {
    let root = r#"<?xml version="1.0" encoding="UTF-8"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="word/document.xml"/></Relationships>"#;
    let rels = r#"<?xml version="1.0" encoding="UTF-8"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rIdImg" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/image" Target="media/fill.png"/></Relationships>"#;
    let doc = format!(r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><w:document {NS}><w:body>{body}</w:body></w:document>"#);
    let mut zw = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    for (name, bytes) in [
        ("_rels/.rels", root.as_bytes()),
        ("word/document.xml", doc.as_bytes()),
        ("word/_rels/document.xml.rels", rels.as_bytes()),
        ("word/media/fill.png", picture),
    ] {
        zw.start_file(name, zip::write::SimpleFileOptions::default()).unwrap();
        zw.write_all(bytes).unwrap();
    }
    wordcraft_docx::read(&zw.finish().unwrap().into_inner()).unwrap()
}

fn saved(d: &Document) -> Document {
    wordcraft_docx::read(&wordcraft_docx::write(d).unwrap()).unwrap()
}

fn paras(d: &Document) -> Vec<&Paragraph> {
    d.body.iter().filter_map(|b| b.as_para()).collect()
}

fn para(pict: &str) -> String {
    format!(r#"<w:p><w:r><w:pict>{pict}</w:pict></w:r></w:p>"#)
}

fn rect(style: &str, attrs: &str, fill: &str) -> String {
    para(&format!(r#"<v:rect style="{style}" {attrs}>{fill}</v:rect>"#))
}

fn text_box(style: &str, attrs: &str, text: &str) -> String {
    para(&format!(
        r##"{TEXT_BOX_TYPE}<v:shape type="#_x0000_t202" style="{style}" {attrs}>{FRAME}<v:textbox><w:txbxContent><w:p><w:r><w:t>{text}</w:t></w:r></w:p></w:txbxContent></v:textbox></v:shape>"##
    ))
}

fn image(o: &InlineObject) -> Option<(String, f32, f32, Wrap, f32, f32, [f32; 4])> {
    match o {
        InlineObject::Image { media, w, h, float, crop, .. } => Some((media.clone(), *w, *h, float.wrap, float.x, float.y, *crop)),
        _ => None,
    }
}

fn on_page(o: &InlineObject) -> bool {
    o.frame().is_some_and(|(_, _, f)| (f.h_rel, f.v_rel, f.x, f.y) == (Anchor::Page, Anchor::Page, 100.0, 100.0))
}

/// What a layer of a picture-filled shape is: the picture (its size and offset), or a shape (its
/// kind, fill, outline and whether it has text).
#[derive(Clone, Debug, PartialEq)]
enum Layer {
    Picture(f32, f32, f32, f32, f32),
    Shape(ShapeKind, Option<Fill>, Option<Rgb>, bool),
}

fn layer(x: f32, y: f32, o: &InlineObject) -> Layer {
    match o {
        InlineObject::Image { w, h, float, .. } => Layer::Picture(x, y, *w, *h, float.rot),
        InlineObject::Shape { kind, fill, stroke, story, .. } => Layer::Shape(*kind, fill.clone(), *stroke, story.is_some()),
        o => panic!("{o:?}"),
    }
}

/// A group's size and its members as layers.
fn layers(o: &InlineObject) -> Option<(f32, f32, Vec<Layer>)> {
    match o {
        InlineObject::Group { w, h, ch_w, ch_h, children, canvas: None, .. } if (ch_w, ch_h) == (w, h) => {
            Some((*w, *h, children.iter().map(|c| layer(c.x, c.y, &c.obj)).collect()))
        }
        _ => None,
    }
}

fn part_text(d: &Document, o: &InlineObject) -> Vec<String> {
    o.text_boxes().iter().flat_map(|id| d.parts[id].blocks.iter().filter_map(|b| b.as_para()).map(|p| p.text.clone())).collect()
}

/// Whether `o` is a group read from a shape filled with a picture.
fn picture_fill(o: &InlineObject) -> Option<bool> {
    match o {
        InlineObject::Group { picture_fill, .. } => Some(*picture_fill),
        _ => None,
    }
}

/// `document.xml` of a saved `d`.
fn written(d: &Document) -> String {
    let bytes = wordcraft_docx::write(d).unwrap();
    let mut zip = zip::ZipArchive::new(std::io::Cursor::new(bytes)).unwrap();
    std::io::read_to_string(zip.by_name("word/document.xml").unwrap()).unwrap()
}

/// Our mark of a group read from a shape filled with a picture, as written: the only extension
/// of its `wp:docPr`.
const MARK: &str =
    r#"><a:extLst><a:ext uri="urn:wordcraft:picture-fill"><wcpf:pictureFill xmlns:wcpf="urn:wordcraft:picture-fill"/></a:ext></a:extLst></wp:docPr>"#;

/// Only the group made for a shape filled with a picture is marked as one, with the fill colour
/// under the picture or not; a `v:group` holding such a shape isn't. Saved, the mark is an
/// extension of the group's `wp:docPr` (the file is otherwise what it is unmarked, the drawing's
/// name and description included) and read back the same.
#[test]
fn only_the_group_read_from_a_picture_fill_is_marked() {
    let member = format!(
        r##"<v:shape type="#_x0000_t202" style="position:absolute;left:0;top:0;width:200;height:100" fillcolor="#00ffff">{FRAME}<v:textbox><w:txbxContent><w:p><w:r><w:t>GRPTEXT</w:t></w:r></w:p></w:txbxContent></v:textbox></v:shape>"##
    );
    let group = format!(
        r#"{TEXT_BOX_TYPE}<v:group style="position:absolute;margin-left:100pt;margin-top:100pt;width:200pt;height:100pt" coordorigin="0,0" coordsize="200,100">{member}</v:group>"#
    );
    let body = [
        text_box(PAGE, "", "boxed"),
        text_box(PAGE, r##"fillcolor="#00ffff""##, "clear"),
        rect(PAGE, r##"fillcolor="#00ffff""##, FRAME),
        para(&group),
    ]
    .concat();
    for (picture, clear) in [(opaque(), false), (see_through(), true)] {
        let d = read(&body, &picture);
        let ps = paras(&d);
        let marks: Vec<_> = ps.iter().map(|p| picture_fill(&p.objects[0])).collect();
        assert_eq!(marks, [Some(true), Some(true), Some(true), Some(false)], "clear {clear}");
        let under = layers(&ps[1].objects[0]).and_then(|(_, _, l)| l.first().cloned());
        let want = if clear { Layer::Shape(ShapeKind::Rectangle, solid(CYAN), None, false) } else { Layer::Picture(0.0, 0.0, 200.0, 100.0, 0.0) };
        assert_eq!(under, Some(want), "clear {clear}: the first member");
        let again = saved(&d);
        let back: Vec<_> = paras(&again).iter().map(|p| picture_fill(&p.objects[0])).collect();
        assert_eq!(back, marks, "clear {clear}: saved");
        for (i, (back, p)) in paras(&again).iter().zip(&ps).enumerate() {
            assert_eq!(layers(&back.objects[0]), layers(&p.objects[0]), "clear {clear}: {i} saved");
        }
        let xml = written(&d);
        assert_eq!(xml.matches(MARK).count(), 3, "clear {clear}: {xml}");
        assert_eq!(xml.matches("urn:wordcraft:picture-fill").count(), 6, "clear {clear}");
        let mut plain = d.clone();
        for b in &mut plain.body {
            if let wordcraft_doc::Block::Para(p) = std::sync::Arc::make_mut(b) {
                for o in &mut p.objects {
                    if let InlineObject::Group { picture_fill, .. } = o {
                        *picture_fill = false;
                    }
                }
            }
        }
        assert_eq!(xml.replace(MARK, "/>"), written(&plain), "clear {clear}: written as unmarked but for the mark");
        assert_eq!(written(&again), xml, "clear {clear}: saved again");
    }
}

/// `d` saved, its `document.xml` changed by `edit`, and read again.
fn reread(d: &Document, edit: impl Fn(String) -> String) -> Result<Document, wordcraft_docx::DocxError> {
    let mut zip = zip::ZipArchive::new(std::io::Cursor::new(wordcraft_docx::write(d).unwrap())).unwrap();
    let mut out = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    for i in 0..zip.len() {
        let mut part = zip.by_index(i).unwrap();
        let name = part.name().to_string();
        let mut bytes = Vec::new();
        std::io::Read::read_to_end(&mut part, &mut bytes).unwrap();
        if name == "word/document.xml" {
            bytes = edit(String::from_utf8(bytes).unwrap()).into_bytes();
        }
        out.start_file(name, zip::write::SimpleFileOptions::default()).unwrap();
        out.write_all(&bytes).unwrap();
    }
    wordcraft_docx::read(&out.finish().unwrap().into_inner())
}

/// The mark's extension as written, and the extension list holding it.
const MARK_EXT: &str = r#"<a:ext uri="urn:wordcraft:picture-fill"><wcpf:pictureFill xmlns:wcpf="urn:wordcraft:picture-fill"/></a:ext>"#;
const MARK_LIST: &str =
    r#"<a:extLst><a:ext uri="urn:wordcraft:picture-fill"><wcpf:pictureFill xmlns:wcpf="urn:wordcraft:picture-fill"/></a:ext></a:extLst>"#;

/// A text box filled with a picture, read as a marked group.
fn marked_text_box() -> Document {
    let d = read(&text_box(PAGE, "", "boxed"), &opaque());
    assert_eq!(picture_fill(&paras(&d)[0].objects[0]), Some(true));
    d
}

/// The first object of `d`'s first paragraph is a group with this mark.
fn first_mark(d: &Document) -> Option<bool> {
    paras(d).first().and_then(|p| p.objects.first()).and_then(picture_fill)
}

/// The mark is read where a file saved again elsewhere may have it: the drawing in
/// `mc:AlternateContent`, the mark after another extension of its `wp:docPr` (the decorative mark,
/// kept too), its element with a namespace declaration of its own.
#[test]
fn the_mark_is_read_beside_other_extensions_in_alternate_content() {
    let decorative = r#"<a:ext uri="{C183D7F6-B498-43B3-948B-1728B52AA6E4}"><adec:decorative xmlns:adec="http://schemas.microsoft.com/office/drawing/2017/decorative" val="1"/></a:ext>"#;
    let again = reread(&marked_text_box(), |xml| {
        assert_eq!(xml.matches(MARK).count(), 1, "{xml}");
        let xml = xml
            .replacen("<w:drawing>", r#"<mc:AlternateContent><mc:Choice Requires="wpg"><w:drawing>"#, 1)
            .replacen("</w:drawing>", "</w:drawing></mc:Choice><mc:Fallback><w:pict/></mc:Fallback></mc:AlternateContent>", 1)
            .replace(MARK_LIST, &format!("<a:extLst>{decorative}{MARK_EXT}</a:extLst>"))
            .replace(r#"xmlns:wcpf="urn:wordcraft:picture-fill"/>"#, r#"xmlns:wcpf="urn:wordcraft:picture-fill" xmlns=""/>"#);
        assert!(xml.contains(r#"<mc:Choice Requires="wpg"><w:drawing>"#) && xml.contains(&format!("<a:extLst>{decorative}<a:ext")), "{xml}");
        xml
    })
    .unwrap();
    let o = &paras(&again)[0].objects[0];
    assert_eq!(picture_fill(o), Some(true), "{o:?}");
    assert!(o.is_decorative());
    assert_eq!(part_text(&again, o), ["boxed"]);
}

/// In `mc:AlternateContent` whose Choice needs what isn't read here, the Fallback is read, and its
/// drawing is marked when its own `wp:docPr` is: a marked drawing in the Fallback, an unmarked
/// one, and an unmarked Fallback behind a marked drawing in the Choice.
#[test]
fn the_mark_of_a_drawing_in_a_fallback() {
    let d = marked_text_box();
    let drawing = |xml: &str| -> String {
        let (Some(a), Some(b)) = (xml.find("<w:drawing>"), xml.find("</w:drawing>")) else { panic!("{xml}") };
        xml[a..b + "</w:drawing>".len()].to_string()
    };
    for (choice_marked, fallback_marked) in [(None, true), (None, false), (Some(true), false)] {
        let again = reread(&d, |xml| {
            let marked = drawing(&xml);
            let plain = marked.replace(MARK, "/>");
            assert_ne!(marked, plain);
            let choice = match choice_marked {
                Some(true) => marked.clone(),
                Some(false) => plain.clone(),
                None => "<w:pict/>".to_string(),
            };
            let fallback = if fallback_marked { &marked } else { &plain };
            xml.replacen(
                &marked,
                &format!(r#"<mc:AlternateContent><mc:Choice Requires="wpx9">{choice}</mc:Choice><mc:Fallback>{fallback}</mc:Fallback></mc:AlternateContent>"#),
                1,
            )
        })
        .unwrap();
        assert_eq!(first_mark(&again), Some(fallback_marked), "choice {choice_marked:?}, fallback {fallback_marked}");
        assert_eq!(part_text(&again, &paras(&again)[0].objects[0]), ["boxed"]);
    }
}

/// A `pictureFill` payload nested `n` elements deep.
fn deep_payload(n: usize) -> String {
    format!(
        r#"<a:extLst><a:ext uri="urn:wordcraft:picture-fill">{}{}</a:ext></a:extLst>"#,
        "<wcpf:x xmlns:wcpf=\"urn:wordcraft:picture-fill\">".repeat(n),
        "</wcpf:x>".repeat(n)
    )
}

/// Only the mark's exact URI on an `a:ext` directly in the first extension list of the group's
/// `wp:docPr` marks it, whatever else the list holds; nothing panics, and a list nested deeper
/// than the XML reader allows makes the file unreadable with its usual limit error.
#[test]
fn the_mark_in_hostile_extension_lists() {
    let other = |n: usize| format!(r#"<a:ext uri="urn:other:{n}"><o:x xmlns:o="urn:other"/></a:ext>"#);
    let others = (0..2000).map(other).collect::<String>();
    let uri = |u: &str| format!(r#"<a:extLst><a:ext uri="{u}"><wcpf:pictureFill xmlns:wcpf="urn:wordcraft:picture-fill"/></a:ext></a:extLst>"#);
    let cases: Vec<(&str, String, bool)> = vec![
        ("as written", MARK_LIST.into(), true),
        ("no list", String::new(), false),
        ("an empty list", "<a:extLst/>".into(), false),
        ("no payload", r#"<a:extLst><a:ext uri="urn:wordcraft:picture-fill"/></a:extLst>"#.into(), true),
        ("after 2000 others", format!("<a:extLst>{others}{MARK_EXT}</a:extLst>"), true),
        ("twice", format!("<a:extLst>{MARK_EXT}{MARK_EXT}</a:extLst>"), true),
        ("in the first of two lists", format!("{MARK_LIST}<a:extLst>{}</a:extLst>", other(1)), true),
        ("in the second of two lists", format!("<a:extLst>{}</a:extLst>{MARK_LIST}", other(1)), false),
        ("nested in another extension", format!(r#"<a:extLst><a:ext uri="urn:other">{MARK_LIST}</a:ext></a:extLst>"#), false),
        ("in another namespace", r#"<a:extLst><o:ext xmlns:o="urn:other" uri="urn:wordcraft:picture-fill"/></a:extLst>"#.into(), false),
        (
            "DrawingML under another prefix",
            r#"<d:extLst xmlns:d="http://schemas.openxmlformats.org/drawingml/2006/main"><d:ext uri="urn:wordcraft:picture-fill"/></d:extLst>"#
                .into(),
            true,
        ),
        ("a payload 200 deep", deep_payload(200), true),
        ("upper case", uri("URN:WORDCRAFT:PICTURE-FILL"), false),
        ("mixed case", uri("urn:WordCraft:picture-fill"), false),
        ("a leading space", uri(" urn:wordcraft:picture-fill"), false),
        ("a trailing space", uri("urn:wordcraft:picture-fill "), false),
        ("a longer URI", uri("urn:wordcraft:picture-fill:v2"), false),
        ("a shorter URI", uri("urn:wordcraft:picture"), false),
        ("empty", uri(""), false),
    ];
    let d = marked_text_box();
    let mut wrong = Vec::new();
    for (what, list, marked) in cases {
        let again = reread(&d, |xml| {
            assert_eq!(xml.matches(MARK_LIST).count(), 1);
            xml.replace(MARK_LIST, &list)
        })
        .unwrap_or_else(|e| panic!("{what}: {e}"));
        if first_mark(&again) != Some(marked) {
            wrong.push(what);
        }
        assert_eq!(part_text(&again, &paras(&again)[0].objects[0]), ["boxed"], "{what}");
    }
    assert_eq!(wrong, Vec::<&str>::new(), "read with the wrong mark");
    let deep = reread(&d, |xml| xml.replace(MARK_LIST, &deep_payload(300)));
    assert!(matches!(deep, Err(wordcraft_docx::DocxError::Limit(_))), "{deep:?}");
}

/// The mark on a drawing that isn't a group does nothing: a picture stays a picture, and a
/// Drawing Canvas, never written with it, isn't marked when it has it.
#[test]
fn the_mark_on_a_picture_or_a_canvas() {
    let picture = read(&rect(PAGE, r#"stroked="f""#, FRAME), &opaque());
    assert!(image(&paras(&picture)[0].objects[0]).is_some());
    let again = reread(&picture, |xml| {
        let xml = xml.replacen(r#"descr=""/><wp:cNvGraphicFramePr"#, &format!(r#"descr="">{MARK_LIST}</wp:docPr><wp:cNvGraphicFramePr"#), 1);
        assert_eq!(xml.matches(MARK_LIST).count(), 1, "{xml}");
        xml
    })
    .unwrap();
    assert!(image(&paras(&again)[0].objects[0]).is_some());
    let mut canvas = marked_text_box();
    for b in &mut canvas.body {
        if let wordcraft_doc::Block::Para(p) = std::sync::Arc::make_mut(b) {
            for o in &mut p.objects {
                if let InlineObject::Group { canvas, .. } = o {
                    *canvas = Some(wordcraft_doc::para::CanvasStyle::default());
                }
            }
        }
    }
    let again = reread(&canvas, |xml| {
        assert!(!xml.contains("urn:wordcraft:picture-fill"), "a canvas isn't written marked");
        let xml = xml.replacen(r#"descr=""/><wp:cNvGraphicFramePr"#, &format!(r#"descr="">{MARK_LIST}</wp:docPr><wp:cNvGraphicFramePr"#), 1);
        assert_eq!(xml.matches(MARK_LIST).count(), 1, "{xml}");
        xml
    })
    .unwrap();
    let o = &paras(&again)[0].objects[0];
    assert!(matches!(o, InlineObject::Group { canvas: Some(_), picture_fill: false, .. }), "{o:?}");
}

/// Word draws the picture stretched over the shape's box, the outline and text over it, and only
/// when the fill has an `o:title`; tiles, patterns, a fill without a type, a shape that isn't
/// filled, a rounded rectangle, a half-transparent fill, a kept aspect, a missing picture and a
/// fill turned off leave the shape as it was. An opaque picture hides the fill colour, so none is
/// kept under it. Saved as a DrawingML picture and groups, they read back the same.
#[test]
fn vml_shapes_filled_with_a_picture_are_drawn_with_it() {
    let body = [
        rect(PAGE, r#"stroked="f""#, FRAME),
        rect(PAGE, "", FRAME),
        rect(PAGE, r#"strokecolor="white [3212]""#, r#"<v:fill type="frame" o:title="" recolor="t" r:id="rIdImg"/>"#),
        text_box(PAGE, "", "boxed"),
        rect("width:100pt;height:50pt", r#"stroked="f""#, FRAME),
        rect(PAGE, "", r#"<v:fill type="frame" r:id="rIdImg"/>"#),
        rect(PAGE, "", r#"<v:fill type="tile" o:title="pic" r:id="rIdImg"/>"#),
        rect(PAGE, r##"fillcolor="#00ffff""##, r#"<v:fill o:title="pic" r:id="rIdImg"/>"#),
        rect(PAGE, r#"filled="f""#, FRAME),
        para(&format!(r#"<v:roundrect style="{PAGE}" arcsize="30%">{FRAME}</v:roundrect>"#)),
        rect(PAGE, "", r#"<v:fill type="frame" o:title="pic" opacity="32768f" r:id="rIdImg"/>"#),
        rect(PAGE, "", r#"<v:fill type="frame" o:title="pic" aspect="atLeast" r:id="rIdImg"/>"#),
        rect(PAGE, "", r#"<v:fill type="frame" o:title="pic" r:id="rIdNone"/>"#),
        rect(PAGE, "", r#"<v:fill type="frame" o:title="pic" on="False" r:id="rIdImg"/>"#),
    ]
    .concat();
    let d = read(&body, &opaque());
    let ps = paras(&d);
    assert_eq!(image(&ps[0].objects[0]), Some(("fill.png".into(), 200.0, 100.0, Wrap::InFrontOfText, 100.0, 100.0, [0.0; 4])));
    assert!(on_page(&ps[0].objects[0]));
    for (i, stroke) in [(1, Rgb::BLACK), (2, Rgb::WHITE)] {
        let want = vec![Layer::Picture(0.0, 0.0, 200.0, 100.0, 0.0), Layer::Shape(ShapeKind::Rectangle, None, Some(stroke), false)];
        assert_eq!(layers(&ps[i].objects[0]), Some((200.0, 100.0, want)), "{i}");
        assert!(on_page(&ps[i].objects[0]), "{i}");
    }
    let boxed = vec![Layer::Picture(0.0, 0.0, 200.0, 100.0, 0.0), Layer::Shape(ShapeKind::TextBox, None, Some(Rgb::BLACK), true)];
    assert_eq!(layers(&ps[3].objects[0]), Some((200.0, 100.0, boxed)));
    assert_eq!(part_text(&d, &ps[3].objects[0]), ["boxed"]);
    assert_eq!(image(&ps[4].objects[0]), Some(("fill.png".into(), 100.0, 50.0, Wrap::Inline, 0.0, 0.0, [0.0; 4])));
    let shape = |i: usize| match &ps[i].objects[0] {
        InlineObject::Shape { kind, fill, stroke, .. } => Some((*kind, fill.clone(), *stroke)),
        _ => None,
    };
    let black = Some(Rgb::BLACK);
    let white = || solid(Rgb::WHITE);
    assert_eq!(shape(5), Some((ShapeKind::Rectangle, white(), black)), "no o:title");
    assert_eq!(shape(6), Some((ShapeKind::Rectangle, white(), black)), "tiled");
    assert_eq!(shape(7), Some((ShapeKind::Rectangle, solid(CYAN), black)), "no type");
    assert_eq!(shape(8), Some((ShapeKind::Rectangle, None, black)), "not filled");
    assert_eq!(shape(9), Some((ShapeKind::RoundedRectangle, white(), black)), "rounded");
    let half = Some(Fill::Solid { color: Rgb::WHITE, transparency: 50.0 });
    assert_eq!(shape(10), Some((ShapeKind::Rectangle, half, black)), "half transparent");
    assert_eq!(shape(11), Some((ShapeKind::Rectangle, white(), black)), "aspect kept");
    assert_eq!(shape(12), Some((ShapeKind::Rectangle, white(), black)), "no picture");
    assert_eq!(shape(13), Some((ShapeKind::Rectangle, None, black)), "fill turned off");
    let again = saved(&d);
    let back = paras(&again);
    assert_eq!(image(&back[0].objects[0]), image(&ps[0].objects[0]));
    assert!(on_page(&back[0].objects[0]));
    for i in 1..4 {
        assert_eq!(layers(&back[i].objects[0]), layers(&ps[i].objects[0]), "{i}");
        assert!(on_page(&back[i].objects[0]), "{i}");
    }
    assert_eq!(part_text(&again, &back[3].objects[0]), ["boxed"]);
    assert_eq!(again.media.values().map(|m| m.as_slice()).collect::<Vec<_>>(), [opaque().as_slice()]);
}

/// Word draws the shape's fill colour under the picture: cyan shows through a picture's
/// transparent half (Word for Mac, a cyan rectangle and text box with a picture whose left half
/// is transparent). Kept for any picture that may be transparent, also after a save; an opaque
/// one (a JPEG, a PNG with neither alpha nor a transparent colour) hides it.
#[test]
fn the_fill_colour_stays_under_a_picture_that_may_be_transparent() {
    let body = [
        rect(PAGE, r##"fillcolor="#00ffff""##, FRAME),
        text_box(PAGE, r##"fillcolor="#00ffff" strokecolor="blue""##, "TBTEXT"),
        rect(PAGE, r##"fillcolor="#00ffff" stroked="f""##, FRAME),
    ]
    .concat();
    let d = read(&body, &see_through());
    let ps = paras(&d);
    let under = Layer::Shape(ShapeKind::Rectangle, solid(CYAN), None, false);
    let picture = || Layer::Picture(0.0, 0.0, 200.0, 100.0, 0.0);
    let want = [
        vec![under.clone(), picture(), Layer::Shape(ShapeKind::Rectangle, None, Some(Rgb::BLACK), false)],
        vec![under.clone(), picture(), Layer::Shape(ShapeKind::TextBox, None, Some(Rgb(0, 0, 0xFF)), true)],
        vec![under.clone(), picture()],
    ];
    for (i, want) in want.into_iter().enumerate() {
        assert_eq!(layers(&ps[i].objects[0]), Some((200.0, 100.0, want)), "{i}");
        assert!(on_page(&ps[i].objects[0]), "{i}");
    }
    assert_eq!(part_text(&d, &ps[1].objects[0]), ["TBTEXT"]);
    let again = saved(&d);
    let back = paras(&again);
    for i in 0..3 {
        assert_eq!(layers(&back[i].objects[0]), layers(&ps[i].objects[0]), "{i} saved");
        assert!(on_page(&back[i].objects[0]), "{i} saved");
    }
    assert_eq!(part_text(&again, &back[1].objects[0]), ["TBTEXT"]);
    let jpeg = [0xFF, 0xD8, 0xFF, 0xE0, 0, 16];
    for (picture, clear) in [
        (jpeg.to_vec(), false),
        (opaque(), false),
        (png(0, &[b"IDAT"]), false),
        (png(3, &[b"PLTE", b"IDAT"]), false),
        (png(3, &[b"PLTE", b"tRNS", b"IDAT"]), true),
        (png(2, &[b"tRNS", b"IDAT"]), true),
        (png(4, &[b"IDAT"]), true),
        (see_through(), true),
        (png(2, &[]), true),
        (b"not a picture".to_vec(), true),
    ] {
        let d = read(&rect(PAGE, r##"fillcolor="#00ffff" stroked="f""##, FRAME), &picture);
        let o = &paras(&d)[0].objects[0];
        assert_eq!(image(o).is_none(), clear, "{picture:?}: {o:?}");
    }
}

/// Word draws the picture upright: a shape turned by a quarter turn has it over its turned box
/// (a group member turned 90° in Word for Mac: its 150 × 50 box drawn 50 × 150, the picture
/// upright in it), half a turn draws it as it is. At other angles Word cuts the upright picture
/// to the turned shape, which isn't followed: the shape keeps its fill, as before.
#[test]
fn turned_shapes_filled_with_a_picture() {
    let turned = |rot: &str| format!("{PAGE};rotation:{rot}");
    let body = [
        text_box(&turned("180"), "", "half"),
        rect(&turned("90"), r#"strokecolor="red""#, FRAME),
        rect(&turned("30"), "", FRAME),
        rect(&format!("{PAGE};flip:x"), "", FRAME),
    ]
    .concat();
    let d = read(&body, &opaque());
    let ps = paras(&d);
    let spin = |o: &InlineObject| o.frame().map(|(_, _, f)| f.rot);
    let half = vec![Layer::Picture(0.0, 0.0, 200.0, 100.0, 180.0), Layer::Shape(ShapeKind::TextBox, None, Some(Rgb::BLACK), true)];
    assert_eq!(layers(&ps[0].objects[0]), Some((200.0, 100.0, half)));
    assert_eq!(spin(&ps[0].objects[0]), Some(180.0));
    let quarter = vec![Layer::Picture(50.0, -50.0, 100.0, 200.0, 270.0), Layer::Shape(ShapeKind::Rectangle, None, Some(Rgb(0xFF, 0, 0)), false)];
    assert_eq!(layers(&ps[1].objects[0]), Some((200.0, 100.0, quarter)));
    assert_eq!(spin(&ps[1].objects[0]), Some(90.0));
    for i in [2, 3] {
        assert!(
            matches!(&ps[i].objects[0], InlineObject::Shape { kind: ShapeKind::Rectangle, fill, .. } if *fill == solid(Rgb::WHITE)),
            "{i}: {:?}",
            ps[i].objects[0]
        );
    }
}

/// A `v:group`'s members filled with a picture: each member's layers are members of the group in
/// its place, the turned one's picture upright over its turned box (Word for Mac: a 300 × 200 pt
/// group with `coordsize` 600,400 draws its members' pictures at 0,0 200 × 100, at 150,75
/// 50 × 150 for the member turned 90° whose box is 150 × 50 at 100,125, and at 210,0 90 × 100
/// for a text box in a nested group).
#[test]
fn group_members_filled_with_a_picture() {
    let member =
        |el: &str, style: &str, attrs: &str, inner: &str| format!(r#"<{el} style="position:absolute;{style}" {attrs}>{FRAME}{inner}</{el}>"#);
    let group = [
        r#"<v:group style="position:absolute;margin-left:100pt;margin-top:100pt;width:300pt;height:200pt" coordorigin="0,0" coordsize="600,400">"#
            .to_string(),
        member("v:rect", "left:0;top:0;width:400;height:200", r#"strokecolor="red" strokeweight="2pt""#, ""),
        member("v:rect", "left:200;top:250;width:300;height:100;rotation:90", r#"stroked="f""#, ""),
        member("v:rect", "left:0;top:250;width:160;height:100;visibility:hidden", r##"fillcolor="#00ffff" stroked="f""##, ""),
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
    for (picture, clear) in [(opaque(), false), (see_through(), true)] {
        let d = read(&para(&format!("{TEXT_BOX_TYPE}{group}")), &picture);
        let InlineObject::Group { w, h, ch_w, ch_h, children, .. } = &paras(&d)[0].objects[0] else { panic!("{:?}", paras(&d)[0].objects) };
        assert_eq!((*w, *h, *ch_w, *ch_h), (300.0, 200.0, 600.0, 400.0));
        let got: Vec<(f32, f32, Layer, f32, bool)> = children
            .iter()
            .map(|c| {
                let f = c.obj.frame().map(|(_, _, f)| (f.rot, f.hidden)).unwrap_or_default();
                (c.x, c.y, layer(0.0, 0.0, &c.obj), f.0, f.1)
            })
            .collect();
        let pic = |w: f32, h: f32| Layer::Picture(0.0, 0.0, w, h, 0.0);
        let mut want = vec![];
        let under = |want: &mut Vec<_>, x, y, rot, hidden, fill| {
            if clear {
                want.push((x, y, Layer::Shape(ShapeKind::Rectangle, fill, None, false), rot, hidden));
            }
        };
        under(&mut want, 0.0, 0.0, 0.0, false, solid(Rgb::WHITE));
        want.push((0.0, 0.0, pic(400.0, 200.0), 0.0, false));
        want.push((0.0, 0.0, Layer::Shape(ShapeKind::Rectangle, None, Some(Rgb(0xFF, 0, 0)), false), 0.0, false));
        under(&mut want, 200.0, 250.0, 90.0, false, solid(Rgb::WHITE));
        want.push((300.0, 150.0, pic(100.0, 300.0), 0.0, false));
        under(&mut want, 0.0, 250.0, 0.0, true, solid(CYAN));
        want.push((0.0, 250.0, pic(160.0, 100.0), 0.0, true));
        under(&mut want, 420.0, 0.0, 0.0, false, solid(CYAN));
        want.push((420.0, 0.0, pic(180.0, 200.0), 0.0, false));
        want.push((420.0, 0.0, Layer::Shape(ShapeKind::TextBox, None, Some(Rgb(0, 0, 0xFF)), true), 0.0, false));
        assert_eq!(got, want, "clear {clear}");
        let text: Vec<String> = children.iter().flat_map(|c| part_text(&d, &c.obj)).collect();
        assert_eq!(text, ["GRPTEXT"]);
    }
}

/// A `v:group` of `n` small rectangles, then `member`, then one more rectangle.
fn group_of(n: usize, member: &str) -> String {
    let rect = |i: usize| format!(r#"<v:rect style="position:absolute;left:{};top:0;width:1;height:1" stroked="f"/>"#, i % 200);
    let rects: String = (0..n).map(rect).collect();
    let last = rect(0);
    para(&format!(
        r#"{TEXT_BOX_TYPE}<v:group style="position:absolute;margin-left:0;margin-top:0;width:200pt;height:200pt" coordorigin="0,0" coordsize="200,200">{rects}{member}{last}</v:group>"#
    ))
}

/// A `v:group` keeps at most `MAX_GROUP_CHILDREN` members, and a member filled with a picture is
/// all its layers or none: after 997, 998 or 999 rectangles, a text box filled with a picture
/// that may be transparent (three layers) or an opaque one (two) is there whole when it fits,
/// else not at all, its story with it; the rectangles before it stay, the one after it doesn't.
#[test]
fn a_picture_fill_at_the_group_limit_is_kept_whole_or_not_at_all() {
    let member = format!(
        r##"<v:shape type="#_x0000_t202" style="position:absolute;left:0;top:100;width:200;height:100" fillcolor="#00ffff">{FRAME}<v:textbox><w:txbxContent><w:p><w:r><w:t>LIMIT</w:t></w:r></w:p></w:txbxContent></v:textbox></v:shape>"##
    );
    let limit = wordcraft_doc::para::MAX_GROUP_CHILDREN;
    let rectangle = Layer::Shape(ShapeKind::Rectangle, solid(Rgb::WHITE), None, false);
    let picture = Layer::Picture(0.0, 0.0, 200.0, 100.0, 0.0);
    let text = Layer::Shape(ShapeKind::TextBox, None, Some(Rgb::BLACK), true);
    let under = Layer::Shape(ShapeKind::Rectangle, solid(CYAN), None, false);
    for (before, clear) in [(limit - 1, true), (limit - 1, false), (limit - 2, true), (limit - 2, false), (limit - 3, true)] {
        let d = read(&group_of(before, &member), &if clear { see_through() } else { opaque() });
        let InlineObject::Group { children, .. } = &paras(&d)[0].objects[0] else { panic!("{:?}", paras(&d)[0].objects) };
        let got: Vec<Layer> = children.iter().map(|c| layer(0.0, 0.0, &c.obj)).collect();
        let whole = if clear { vec![under.clone(), picture.clone(), text.clone()] } else { vec![picture.clone(), text.clone()] };
        let fits = before + whole.len() <= limit;
        let mut want = vec![rectangle.clone(); before];
        if fits {
            want.extend(whole);
        }
        assert_eq!(got.len(), want.len(), "{before} before, clear {clear}");
        assert!(got == want, "{before} before, clear {clear}: the member is partial: {:?}", got.get(before..));
        let boxes = d.parts.values().filter(|p| p.kind == wordcraft_doc::PartKind::TextBox).count();
        assert_eq!(boxes, usize::from(fits), "{before} before, clear {clear}: its story");
    }
}
