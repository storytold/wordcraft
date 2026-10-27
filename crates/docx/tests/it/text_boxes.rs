//! Text boxes' text direction, alignment and links (`wps:bodyPr/@vert`, `@anchor`;
//! `wps:txbx/@id`, `wps:linkedTxbx`). Fixtures are hand-written from ECMA-376 Part 1 §20.4
//! (DrawingML WordprocessingML drawing) and §21.1.2.1.1 (`bodyPr`).

use std::io::{Read, Write};

use wordcraft_doc::para::ShapeKind;
use wordcraft_doc::props::{TextVert, VAlign};
use wordcraft_doc::{Document, InlineObject, PartKind, TextBody, para_block};

fn zip(entries: &[(&str, &[u8])]) -> Vec<u8> {
    let mut zw = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    for (name, bytes) in entries {
        zw.start_file(*name, zip::write::SimpleFileOptions::default()).unwrap();
        zw.write_all(bytes).unwrap();
    }
    zw.finish().unwrap().into_inner()
}

fn document_xml(bytes: &[u8]) -> String {
    let mut z = zip::ZipArchive::new(std::io::Cursor::new(bytes)).unwrap();
    let mut s = String::new();
    z.by_name("word/document.xml").unwrap().read_to_string(&mut s).unwrap();
    s
}

/// The text boxes' stories in body order, a group's members among them
/// (`text_boxes`, not `text_box`, which only sees an object that is itself a box).
fn boxes(d: &Document) -> Vec<u32> {
    let mut out = Vec::new();
    d.objects_in_reading_order(&mut |o| out.extend(o.text_boxes()));
    out.retain(|id| d.parts.get(id).is_some_and(|p| p.kind == PartKind::TextBox));
    out
}

fn text_box(d: &mut Document, text: &str, body: TextBody) -> InlineObject {
    let id = d.add_part(PartKind::TextBox, vec![para_block(wordcraft_doc::Paragraph::with_text(text, Default::default()))]);
    d.parts.get_mut(&id).unwrap().body = body;
    InlineObject::Shape {
        kind: ShapeKind::TextBox,
        w: 144.0,
        h: 72.0,
        fill: None,
        stroke: None,
        stroke_width: 0.75,
        float: Default::default(),
        story: Some(id),
        effects: Default::default(),
        freeform: None,
        extra: Default::default(),
    }
}

#[test]
fn direction_alignment_and_links_round_trip() {
    let mut d = Document::from_text("a\nb\nc");
    let a = text_box(&mut d, "Flows on", TextBody { vert: TextVert::Vert, anchor: VAlign::Center, ..Default::default() });
    let b = text_box(&mut d, "", TextBody::default());
    let c = text_box(&mut d, "", TextBody { vert: TextVert::Stacked, anchor: VAlign::Bottom, ..Default::default() });
    for (i, o) in [a, b, c].into_iter().enumerate() {
        d.insert_object(&wordcraft_doc::Pos::body(i, 1), o, &Default::default()).unwrap();
    }
    let ids = boxes(&d);
    d.parts.get_mut(&ids[0]).unwrap().body.next = Some(ids[1]);
    d.parts.get_mut(&ids[1]).unwrap().body.next = Some(ids[2]);
    let bytes = wordcraft_docx::write(&d).unwrap();
    let xml = document_xml(&bytes);
    assert!(xml.contains(r#"<wps:txbx id="1">"#), "{xml}");
    assert!(xml.contains(r#"<wps:linkedTxbx id="1" seq="1"/>"#) && xml.contains(r#"<wps:linkedTxbx id="1" seq="2"/>"#));
    assert!(xml.contains(r#"<wps:bodyPr vert="vert" anchor="ctr"/>"#) && xml.contains(r#"<wps:bodyPr vert="wordArtVert" anchor="b"/>"#));
    let r = wordcraft_docx::read(&bytes).unwrap();
    let got = boxes(&r);
    assert_eq!(r.text_box_chains(), vec![got.clone()]);
    assert_eq!(r.plain_text(wordcraft_doc::StoryRef::Part(got[0])), "Flows on");
    let body = |id: u32| r.parts.get(&id).unwrap().body;
    assert_eq!((body(got[0]).vert, body(got[0]).anchor), (TextVert::Vert, VAlign::Center));
    assert_eq!((body(got[2]).vert, body(got[2]).anchor), (TextVert::Stacked, VAlign::Bottom));
}

/// A paragraph with an inline text box whose `wps:wsp` children are `inner`.
fn wsp(inner: &str) -> String {
    format!(
        r#"<w:p><w:r><w:drawing><wp:inline><wp:extent cx="1828800" cy="914400"/><wp:docPr id="1" name="Box"/><a:graphic><a:graphicData uri="http://schemas.microsoft.com/office/word/2010/wordprocessingShape"><wps:wsp><wps:cNvSpPr txBox="1"/><wps:spPr><a:prstGeom prst="rect"/></wps:spPr>{inner}</wps:wsp></a:graphicData></a:graphic></wp:inline></w:drawing></w:r></w:p>"#
    )
}

#[test]
fn hostile_links_read_without_chains_running_wild() {
    let head = wsp(
        r#"<wps:txbx id="7"><w:txbxContent><w:p><w:r><w:t>Text</w:t></w:r></w:p></w:txbxContent></wps:txbx><wps:bodyPr vert="eaVert" anchor="dist"/>"#,
    );
    // A repeated seq, a link to a chain with no first box, a seq that isn't a number.
    let body = [
        head,
        wsp(r#"<wps:linkedTxbx id="7" seq="1"/><wps:bodyPr/>"#),
        wsp(r#"<wps:linkedTxbx id="7" seq="1"/><wps:bodyPr/>"#),
        wsp(r#"<wps:linkedTxbx id="9" seq="1"/><wps:bodyPr/>"#),
        wsp(r#"<wps:linkedTxbx id="7" seq="-3"/><wps:bodyPr/>"#),
    ]
    .concat();
    let root = r#"<?xml version="1.0" encoding="UTF-8"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="word/document.xml"/></Relationships>"#;
    let doc = format!(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main" xmlns:wp="http://schemas.openxmlformats.org/drawingml/2006/wordprocessingDrawing" xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" xmlns:wps="http://schemas.microsoft.com/office/word/2010/wordprocessingShape"><w:body>{body}</w:body></w:document>"#
    );
    let r = wordcraft_docx::read(&zip(&[("_rels/.rels", root.as_bytes()), ("word/document.xml", doc.as_bytes())])).unwrap();
    let got = boxes(&r);
    assert_eq!(got.len(), 4, "every box with a story is read (a bad seq links nothing)");
    assert_eq!(r.text_box_chains(), vec![vec![got[0], got[1]]], "one chain: the first box and the first seq 1");
    let b = r.parts.get(&got[0]).unwrap().body;
    assert_eq!((b.vert, b.anchor), (TextVert::Vert, VAlign::Center));
}

#[test]
fn vml_text_boxes_keep_their_inset_fit_and_anchor() {
    // ECMA-376 Part 4 §19.1.2.22: `inset="l,t,r,b"`, each one left out keeping the default
    // (0.1in, 0.05in); `mso-fit-shape-to-text` resizes the shape to its text; `v-text-anchor`
    // places the text between the box's top and bottom.
    let content = r#"<w:txbxContent><w:p><w:r><w:t>Boxed</w:t></w:r></w:p></w:txbxContent>"#;
    let shape = |style: &str, textbox: &str| {
        format!(
            r#"<w:p><w:r><w:pict><v:shape style="width:100pt;height:40pt;{style}"><v:textbox {textbox}>{content}</v:textbox></v:shape></w:pict></w:r></w:p>"#
        )
    };
    let body = [
        shape("", r#"inset="0,0,0,0""#),
        shape("v-text-anchor:middle", r#"inset="1mm,,2pt" style="mso-fit-shape-to-text:T""#),
        shape("", ""),
        // A bare number is in EMUs; one that doesn't parse keeps the default; negative is 0;
        // too large is the most a margin may be.
        shape("v-text-anchor:bottom-baseline", r#"inset="12700,bogus,-5pt,99in""#),
        // `o:insetmode="auto"` leaves the margins to the application.
        shape("", r#"o:insetmode="auto" inset="0,0,0,0""#),
        // In a group, a member's margins go with that member's text, not an empty box's before it.
        format!(r#"<w:p><w:r><w:pict><v:group style="width:100pt;height:40pt"><v:shape><v:textbox inset="9pt,9pt,9pt,9pt"/></v:shape><v:shape><v:textbox inset="0,0,0,0">{content}</v:textbox></v:shape></v:group></w:pict></w:r></w:p>"#),
    ]
    .concat();
    let doc = format!(
        r#"<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main" xmlns:v="urn:schemas-microsoft-com:vml" xmlns:o="urn:schemas-microsoft-com:office:office"><w:body>{body}</w:body></w:document>"#
    );
    let rels = br#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="word/document.xml"/></Relationships>"#;
    let d = wordcraft_docx::read(&zip(&[("_rels/.rels", rels), ("word/document.xml", doc.as_bytes())])).unwrap();
    let bodies: Vec<TextBody> = boxes(&d).iter().map(|id| d.parts[id].body).collect();
    assert_eq!(bodies.len(), 6);
    assert_eq!(bodies[0].inset, Some([0, 0, 0, 0]));
    assert_eq!((bodies[1].inset, bodies[1].auto_fit, bodies[1].anchor), (Some([36_000, 45_720, 25_400, 45_720]), true, VAlign::Center));
    assert_eq!(bodies[2], TextBody::default());
    assert_eq!((bodies[3].inset, bodies[3].anchor), (Some([12_700, 45_720, 0, wordcraft_doc::MAX_TEXT_BOX_INSET]), VAlign::Bottom));
    assert_eq!(bodies[4].inset, None);
    assert_eq!(bodies[5].inset, Some([0, 0, 0, 0]));
    // Saved and read again, they stay.
    let back = wordcraft_docx::read(&wordcraft_docx::write(&d).unwrap()).unwrap();
    let again: Vec<TextBody> = boxes(&back).iter().map(|id| back.parts[id].body).collect();
    assert_eq!(again, bodies);
}

#[test]
fn hidden_drawings_stay_hidden() {
    // VML `visibility:hidden` (floating or inline) and DrawingML `wp:docPr/@hidden` (also a group
    // member's `wps:cNvPr/@hidden`) mark a drawing hidden; it is kept, and saved hidden.
    let vml = r#"<w:p><w:r><w:pict><v:shape style="position:absolute;width:50pt;height:50pt;visibility:hidden"><v:textbox><w:txbxContent><w:p/></w:txbxContent></v:textbox></v:shape></w:pict></w:r></w:p>"#;
    let shown = r#"<w:p><w:r><w:pict><v:shape style="position:absolute;width:50pt;height:50pt;visibility:visible"><v:textbox><w:txbxContent><w:p/></w:txbxContent></v:textbox></v:shape></w:pict></w:r></w:p>"#;
    let inline_vml = r#"<w:p><w:r><w:pict><v:rect style="width:20pt;height:20pt;visibility:hidden"/></w:pict></w:r></w:p>"#;
    let dml = r#"<w:p><w:r><w:drawing><wp:inline><wp:extent cx="635000" cy="635000"/><wp:docPr id="7" name="Box" hidden="1"/><a:graphic><a:graphicData uri="http://schemas.microsoft.com/office/word/2010/wordprocessingShape"><wps:wsp><wps:spPr><a:prstGeom prst="rect"/></wps:spPr><wps:txbx><w:txbxContent><w:p/></w:txbxContent></wps:txbx><wps:bodyPr/></wps:wsp></a:graphicData></a:graphic></wp:inline></w:drawing></w:r></w:p>"#;
    let member = |id: u32, hidden: &str| {
        format!(
            r#"<wps:wsp><wps:cNvPr id="{id}" name="M{id}"{hidden}/><wps:cNvSpPr/><wps:spPr><a:xfrm><a:off x="0" y="0"/><a:ext cx="635000" cy="635000"/></a:xfrm><a:prstGeom prst="rect"/></wps:spPr><wps:bodyPr/></wps:wsp>"#
        )
    };
    let group = format!(
        r#"<w:p><w:r><w:drawing><wp:inline><wp:extent cx="1270000" cy="635000"/><wp:docPr id="8" name="Group"/><a:graphic><a:graphicData uri="http://schemas.microsoft.com/office/word/2010/wordprocessingGroup"><wpg:wgp><wpg:cNvGrpSpPr/><wpg:grpSpPr><a:xfrm><a:off x="0" y="0"/><a:ext cx="1270000" cy="635000"/><a:chOff x="0" y="0"/><a:chExt cx="1270000" cy="635000"/></a:xfrm></wpg:grpSpPr>{}{}</wpg:wgp></a:graphicData></a:graphic></wp:inline></w:drawing></w:r></w:p>"#,
        member(9, r#" hidden="1""#),
        member(10, "")
    );
    let doc = format!(
        r#"<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main" xmlns:v="urn:schemas-microsoft-com:vml" xmlns:wp="http://schemas.openxmlformats.org/drawingml/2006/wordprocessingDrawing" xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" xmlns:wps="http://schemas.microsoft.com/office/word/2010/wordprocessingShape" xmlns:wpg="http://schemas.microsoft.com/office/word/2010/wordprocessingGroup"><w:body>{vml}{shown}{inline_vml}{dml}{group}</w:body></w:document>"#
    );
    let rels = br#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="word/document.xml"/></Relationships>"#;
    // Each drawing's flag, a group's members' after it.
    let hidden = |d: &Document| {
        let mut out = Vec::new();
        d.objects_in_reading_order(&mut |o| {
            out.extend(o.frame().map(|(_, _, f)| f.hidden));
            if let InlineObject::Group { children, .. } = o {
                out.extend(children.iter().filter_map(|c| c.obj.frame()).map(|(_, _, f)| f.hidden));
            }
        });
        out
    };
    let want = [true, false, true, true, false, true, false];
    let d = wordcraft_docx::read(&zip(&[("_rels/.rels", rels), ("word/document.xml", doc.as_bytes())])).unwrap();
    assert_eq!(hidden(&d), want);
    let saved = wordcraft_docx::write(&d).unwrap();
    let xml = document_xml(&saved);
    let marked =
        |tag: &str| xml.split(&format!("<{tag} ")).skip(1).filter(|a| a.split('>').next().is_some_and(|a| a.contains(r#"hidden="1""#))).count();
    assert_eq!((marked("wp:docPr"), marked("wps:cNvPr")), (3, 1), "{xml}");
    assert_eq!(hidden(&wordcraft_docx::read(&saved).unwrap()), want);
}

#[test]
fn picture_in_box_keeps_its_frame() {
    use wordcraft_doc::{Paragraph, Pos, StoryRef};

    let mut d = Document::from_text("Anchor");
    let body = TextBody { anchor: VAlign::Center, inset: Some([0; 4]), ..Default::default() };
    let outer = text_box(&mut d, "Label", body);
    let id = outer.text_box().unwrap();
    let mut bytes = std::io::Cursor::new(Vec::new());
    image::RgbaImage::from_pixel(10, 4, image::Rgba([20, 40, 60, 255])).write_to(&mut bytes, image::ImageFormat::Png).unwrap();
    let media = d.add_media(bytes.into_inner(), "png");
    let image = InlineObject::Image {
        media: media.clone(),
        w: 90.0,
        h: 36.0,
        alt: "Synthetic marker".into(),
        float: Default::default(),
        crop: [0.0; 4],
        ole: None,
    };
    let mut p = Paragraph::new();
    p.insert_object(0, image, &Default::default()).unwrap();
    d.parts.get_mut(&id).unwrap().blocks.push(para_block(p));
    d.insert_object(&Pos::body(0, 0), outer, &Default::default()).unwrap();

    let read = wordcraft_docx::read(&wordcraft_docx::write(&d).unwrap()).unwrap();
    let ids = boxes(&read);
    assert_eq!(ids.len(), 1, "a nested picture must not replace its text box");
    let id = ids[0];
    assert_eq!(read.parts[&id].body, body);
    assert!(read.plain_text(StoryRef::Part(id)).starts_with("Label"));
    let p = read.para(StoryRef::Part(id), &wordcraft_doc::Path::top(1)).unwrap();
    assert!(matches!(&p.objects[0], InlineObject::Image { w, h, alt, .. }
        if *w == 90.0 && *h == 36.0 && alt == "Synthetic marker"));
    let p = read.para(StoryRef::Body, &wordcraft_doc::Path::top(0)).unwrap();
    assert!(matches!(&p.objects[0], InlineObject::Shape { w, h, .. } if *w == 144.0 && *h == 72.0));
}

/// A text box resizing to fit its text counts its outline's width, drawn or not (Word: a 0.5 pt
/// `a:ln` with `a:noFill` makes it 0.5 pt taller); a VML box that isn't stroked (`stroked="f"`)
/// has none (Word: no taller than its text and margins).
#[test]
fn text_boxes_keep_the_width_of_their_outline_drawn_or_not() {
    let content = r#"<w:txbxContent><w:p><w:r><w:t>Boxed</w:t></w:r></w:p></w:txbxContent>"#;
    let vml = |attrs: &str| {
        format!(
            r#"<w:p><w:r><w:pict><v:rect style="width:100pt;height:40pt" {attrs}><v:textbox style="mso-fit-shape-to-text:t">{content}</v:textbox></v:rect></w:pict></w:r></w:p>"#
        )
    };
    let dml = |ln: &str| {
        format!(
            r#"<w:p><w:r><w:drawing><wp:inline><wp:extent cx="1270000" cy="508000"/><wp:docPr id="1" name="Box"/><a:graphic><a:graphicData uri="http://schemas.microsoft.com/office/word/2010/wordprocessingShape"><wps:wsp><wps:cNvSpPr txBox="1"/><wps:spPr><a:prstGeom prst="rect"/>{ln}</wps:spPr><wps:txbx>{content}</wps:txbx><wps:bodyPr><a:spAutoFit/></wps:bodyPr></wps:wsp></a:graphicData></a:graphic></wp:inline></w:drawing></w:r></w:p>"#
        )
    };
    let body = [
        dml(r#"<a:ln w="6350"><a:noFill/></a:ln>"#),
        dml(r#"<a:ln w="25400"><a:solidFill><a:srgbClr val="000000"/></a:solidFill></a:ln>"#),
        dml(""),
        vml(r#"stroked="f" strokeweight="2pt""#),
        vml(r#"stroked="f""#),
        vml(""),
        vml(r#"strokeweight="2pt""#),
    ]
    .concat();
    let doc = format!(
        r#"<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main" xmlns:wp="http://schemas.openxmlformats.org/drawingml/2006/wordprocessingDrawing" xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" xmlns:wps="http://schemas.microsoft.com/office/word/2010/wordprocessingShape" xmlns:v="urn:schemas-microsoft-com:vml"><w:body>{body}</w:body></w:document>"#
    );
    let rels = br#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="word/document.xml"/></Relationships>"#;
    let d = wordcraft_docx::read(&zip(&[("_rels/.rels", rels), ("word/document.xml", doc.as_bytes())])).unwrap();
    let outlines = |d: &Document| {
        let mut out = Vec::new();
        d.objects_in_reading_order(&mut |o| {
            if let InlineObject::Shape { stroke, stroke_width, story: Some(id), .. } = o
                && d.parts.get(id).is_some_and(|p| p.body.auto_fit)
            {
                out.push((stroke.is_some(), *stroke_width));
            }
        });
        out
    };
    let read = outlines(&d);
    assert_eq!(read, [(false, 0.5), (true, 2.0), (false, 0.0), (false, 0.0), (false, 0.0), (true, 0.75), (true, 2.0)]);
    let bytes = wordcraft_docx::write(&d).unwrap();
    let xml = document_xml(&bytes);
    assert!(xml.contains(r#"<a:ln w="6350"><a:noFill/></a:ln>"#), "an outline not drawn keeps its width: {xml}");
    assert_eq!(outlines(&wordcraft_docx::read(&bytes).unwrap()), read, "saved and read again");
}

#[test]
fn story_shared_by_many_shapes_is_written_once() {
    let mut d = Document::from_text("x");
    let text = "abcdefghij".repeat(5_000);
    let o = text_box(&mut d, &text, TextBody::default());
    for _ in 0..500 {
        d.insert_object(&wordcraft_doc::Pos::body(0, 0), o.clone(), &Default::default()).unwrap();
    }
    let xml = document_xml(&wordcraft_docx::write(&d).unwrap());
    assert!(xml.len() < 2_000_000, "document.xml is {} bytes", xml.len());
    assert_eq!(xml.matches(text.as_str()).count(), 1);
}
