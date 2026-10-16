//! Legacy VML drawings in a `w:pict`: every shape in one, and `v:group` members in the space
//! `coordsize`/`coordorigin` give them. Fixtures are hand-written from ECMA-376 Part 4
//! (VML, §14.1 `v:group`, `coordsize`, `coordorigin`; §14.2 shapes).

use std::io::Write;

use wordcraft_doc::para::ShapeKind;
use wordcraft_doc::{Document, InlineObject, Paragraph};

fn zip(entries: &[(&str, &[u8])]) -> Vec<u8> {
    let mut zw = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    for (name, bytes) in entries {
        zw.start_file(*name, zip::write::SimpleFileOptions::default()).unwrap();
        zw.write_all(bytes).unwrap();
    }
    zw.finish().unwrap().into_inner()
}

/// A document whose body is `body`.
fn read_body(body: &str) -> Document {
    let root = r#"<?xml version="1.0" encoding="UTF-8"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="word/document.xml"/></Relationships>"#;
    let doc = format!(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main" xmlns:v="urn:schemas-microsoft-com:vml" xmlns:o="urn:schemas-microsoft-com:office:office" xmlns:w10="urn:schemas-microsoft-com:office:word"><w:body>{body}</w:body></w:document>"#
    );
    wordcraft_docx::read(&zip(&[("_rels/.rels", root.as_bytes()), ("word/document.xml", doc.as_bytes())])).unwrap()
}

fn paras(d: &Document) -> Vec<&Paragraph> {
    d.body.iter().filter_map(|b| b.as_para()).collect()
}

/// The objects of the body's first paragraph.
fn objects(d: &Document) -> &[InlineObject] {
    &paras(d).first().expect("a paragraph").objects
}

fn pict(inner: &str) -> String {
    format!(r#"<w:p><w:r><w:pict>{inner}</w:pict></w:r></w:p>"#)
}

fn kinds(objs: &[InlineObject]) -> Vec<ShapeKind> {
    objs.iter()
        .map(|o| match o {
            InlineObject::Shape { kind, .. } => *kind,
            o => panic!("{o:?}"),
        })
        .collect()
}

#[test]
fn every_shape_in_a_pict_is_read() {
    // A `w:pict` may hold any number of shapes; each one is its own object. A `v:shapetype` is
    // a definition, not a shape, and is not one of them.
    let d = read_body(&pict(concat!(
        r#"<v:shapetype id="t1" o:spt="202"/>"#,
        r#"<v:rect style="width:10pt;height:20pt"/>"#,
        r#"<v:oval style="width:30pt;height:40pt"/>"#,
        r#"<v:roundrect style="width:50pt;height:60pt"/>"#,
    )));
    let objs = objects(&d);
    assert_eq!(kinds(objs), [ShapeKind::Rectangle, ShapeKind::Ellipse, ShapeKind::RoundedRectangle], "all three, in document order");
    let size = |o: &InlineObject| match o {
        InlineObject::Shape { w, h, .. } => (*w, *h),
        o => panic!("{o:?}"),
    };
    assert_eq!(objs.iter().map(size).collect::<Vec<_>>(), [(10.0, 20.0), (30.0, 40.0), (50.0, 60.0)], "each keeps its own size");
}

#[test]
fn a_vml_group_keeps_its_members() {
    // `coordsize` is the members' space, mapped onto the group's own box; `left`/`top` place a
    // member in that space.
    let d = read_body(&pict(concat!(
        r#"<v:group style="width:100pt;height:50pt" coordsize="1000,500">"#,
        r#"<v:rect style="left:0;top:0;width:400;height:500"/>"#,
        r#"<v:oval style="left:600;top:100;width:400;height:300"/>"#,
        r#"</v:group>"#,
    )));
    let InlineObject::Group { w, h, ch_w, ch_h, children, canvas, .. } = &objects(&d)[0] else { panic!("{:?}", objects(&d)) };
    assert_eq!((*w, *h, *ch_w, *ch_h), (100.0, 50.0, 1000.0, 500.0), "its own box, and its members' space");
    assert_eq!(*canvas, None, "a group is not a Drawing Canvas");
    assert_eq!(children.len(), 2, "both members");
    assert_eq!((children[0].x, children[0].y), (0.0, 0.0));
    assert_eq!((children[1].x, children[1].y), (600.0, 100.0), "placed in the members' space, not scaled");
    assert_eq!(kinds(&children.iter().map(|c| c.obj.clone()).collect::<Vec<_>>()), [ShapeKind::Rectangle, ShapeKind::Ellipse]);
}

#[test]
fn a_vml_group_shifts_its_members_by_its_coordorigin() {
    // `coordorigin` is the top-left of the members' space, so a member's offset is from there.
    let d = read_body(&pict(concat!(
        r#"<v:group style="width:100pt;height:100pt" coordsize="1000,1000" coordorigin="200,300">"#,
        r#"<v:rect style="left:200;top:300;width:100;height:100"/>"#,
        r#"<v:rect style="left:700;top:800;width:100;height:100"/>"#,
        r#"</v:group>"#,
    )));
    let InlineObject::Group { ch_w, children, .. } = &objects(&d)[0] else { panic!() };
    assert_eq!(*ch_w, 1000.0);
    assert_eq!((children[0].x, children[0].y), (0.0, 0.0), "a member at the origin sits at the group's corner");
    assert_eq!((children[1].x, children[1].y), (500.0, 500.0));
}

#[test]
fn nested_vml_groups_are_flattened_into_the_outer_one() {
    // An inner group's members' space maps onto the inner group's own box, which is itself in
    // the outer group's space: 500 wide in a 1000-wide space inside a 1000-wide box halves it.
    let d = read_body(&pict(concat!(
        r#"<v:group style="width:100pt;height:100pt" coordsize="1000,1000">"#,
        r#"<v:rect style="left:100;top:100;width:50;height:50"/>"#,
        r#"<v:group style="left:400;top:400;width:500;height:500" coordsize="1000,1000">"#,
        r#"<v:oval style="left:200;top:600;width:100;height:100"/>"#,
        r#"</v:group>"#,
        r#"</v:group>"#,
    )));
    let InlineObject::Group { children, .. } = &objects(&d)[0] else { panic!() };
    assert_eq!(children.len(), 2, "the inner group's member comes into the outer group");
    assert_eq!((children[0].x, children[0].y), (100.0, 100.0));
    // 400 + 200 × (500/1000) = 500; 400 + 600 × 0.5 = 700.
    assert_eq!((children[1].x, children[1].y), (500.0, 700.0), "mapped through the inner group");
    let InlineObject::Shape { w, h, .. } = children[1].obj else { panic!() };
    assert_eq!((w, h), (50.0, 50.0), "and scaled by it");
}

#[test]
fn a_vml_group_with_no_member_we_can_show_is_left_out() {
    // Only a shape definition, and a shape we draw nothing for (no stroke, no fill).
    let d = read_body(&pict(concat!(
        r#"<v:group style="width:100pt;height:50pt" coordsize="1000,500">"#,
        r#"<v:shapetype id="t1" o:spt="202"/>"#,
        r#"<v:rect style="left:0;top:0;width:10;height:10" stroked="f" filled="f"/>"#,
        r#"</v:group>"#,
    )));
    assert!(objects(&d).is_empty(), "nothing to show: {:?}", objects(&d));
}

#[test]
fn deeply_nested_vml_groups_stop_at_the_depth_bound() {
    // A hostile file nests groups without end; reading one must not run away.
    let depth = 64;
    let mut inner = r#"<v:rect style="left:0;top:0;width:10;height:10"/>"#.to_string();
    for _ in 0..depth {
        inner = format!(r#"<v:group style="left:0;top:0;width:100;height:100" coordsize="100,100">{inner}</v:group>"#);
    }
    let d = read_body(&pict(&format!(r#"<v:group style="width:100pt;height:100pt" coordsize="100,100">{inner}</v:group>"#)));
    // Past the bound nothing is read, so the outermost group has no member and is left out.
    assert!(objects(&d).is_empty(), "{:?}", objects(&d));
}

#[test]
fn a_vml_group_survives_being_saved() {
    let body = pict(concat!(
        r#"<v:group style="width:100pt;height:50pt" coordsize="1000,500">"#,
        r#"<v:rect style="left:0;top:0;width:400;height:500"/>"#,
        r#"<v:oval style="left:600;top:100;width:400;height:300"/>"#,
        r#"</v:group>"#,
    ));
    let d = read_body(&body);
    let InlineObject::Group { children, .. } = &objects(&d)[0] else { panic!("{:?}", objects(&d)) };
    assert_eq!(children.len(), 2, "read with both its members to begin with");
    let back = wordcraft_docx::read(&wordcraft_docx::write(&d).unwrap()).unwrap();
    assert_eq!(objects(&back), objects(&d), "saved as DrawingML and read again, it is the same group");
}

#[test]
fn a_pict_of_a_rect_a_text_box_and_a_group_all_come_through() {
    // The three shapes of issue #1643, which a round trip dropped the last of. The group's
    // member gives its size in points, not in the group's own coordinates, as Word writes it.
    let d = read_body(&format!(
        "{}{}{}",
        pict(r##"<v:rect style="position:absolute;width:100pt;height:50pt" fillcolor="#ff0000"/>"##),
        pict(
            r#"<v:shape style="width:100pt;height:50pt"><v:textbox><w:txbxContent><w:p><w:r><w:t>boxed</w:t></w:r></w:p></w:txbxContent></v:textbox></v:shape>"#
        ),
        pict(concat!(
            r#"<v:group style="width:100pt;height:50pt" coordsize="100,50">"#,
            r##"<v:rect style="width:50pt;height:20pt" fillcolor="#00ff00"/>"##,
            r#"</v:group>"#,
        )),
    ));
    let ps = paras(&d);
    assert_eq!(ps.len(), 3, "a paragraph each");
    assert_eq!(kinds(&ps[0].objects), [ShapeKind::Rectangle]);
    assert_eq!(kinds(&ps[1].objects), [ShapeKind::TextBox]);
    let InlineObject::Group { children, .. } = &ps[2].objects[0] else { panic!("{:?}", ps[2].objects) };
    assert_eq!(kinds(&children.iter().map(|c| c.obj.clone()).collect::<Vec<_>>()), [ShapeKind::Rectangle], "the green rectangle is read");
    let InlineObject::Shape { fill, .. } = children[0].obj else { panic!() };
    assert_eq!(fill, Some(wordcraft_doc::Rgb(0x00, 0xff, 0x00)), "with its colour");
    // And it is still there after a save, which is what the round trip lost.
    let back = wordcraft_docx::read(&wordcraft_docx::write(&d).unwrap()).unwrap();
    assert_eq!(paras(&back).iter().map(|p| p.objects.len()).collect::<Vec<_>>(), [1, 1, 1]);
    let InlineObject::Group { children, .. } = &paras(&back)[2].objects[0] else { panic!() };
    assert_eq!(children.len(), 1, "the group keeps its member");
}

/// The text of text box story `id`, its paragraphs joined by newlines.
fn part_text(d: &Document, id: u32) -> String {
    d.parts.get(&id).map(|p| p.blocks.iter().filter_map(|b| b.as_para()).map(|p| p.plain_text()).collect::<Vec<_>>().join("\n")).unwrap_or_default()
}

#[test]
fn a_group_of_several_text_boxes_keeps_every_one() {
    // Reported on #1643 by @KNUFFIXX, whose fixture is a `_x0000_t202` pair side by side in one
    // inline `v:group`: only box A came through a convert. `vml_textbox` recurses to the first
    // box with content, so before this every later box in a group was dropped -- and a text box
    // is the one member a group used to keep, which is why this case looked half-working.
    let shape = |left: u32, height: u32, label: &str| {
        let paras: String = (1..=3).map(|n| format!(r#"<w:p><w:r><w:t xml:space="preserve">{label} line {n}</w:t></w:r></w:p>"#)).collect();
        format!(
            r##"<v:shape style="position:absolute;left:{left};top:100;width:3500;height:{height}" type="#_x0000_t202" filled="false" stroked="true"><v:textbox inset="0,0,0,0"><w:txbxContent>{paras}</w:txbxContent></v:textbox></v:shape>"##
        )
    };
    // The control fixture, and the overflow one where box B is too short for its text; the
    // height of a box must not decide whether it is read.
    for (height_b, label_b) in [(1600, "Box B (tall)"), (400, "Box B (short)")] {
        let d = read_body(&pict(&format!(
            concat!(
                r##"<v:shapetype id="_x0000_t202" coordsize="21600,21600" o:spt="202" path="m,l,21600r21600,l21600,xe"/>"##,
                r##"<v:group style="width:400pt;height:100pt" coordorigin="0,0" coordsize="8000,2000">{}{}</v:group>"##,
            ),
            shape(100, 1600, "Box A (tall)"),
            shape(4200, height_b, label_b),
        )));
        let InlineObject::Group { ch_w, ch_h, children, .. } = &objects(&d)[0] else { panic!("{:?}", objects(&d)) };
        assert_eq!((*ch_w, *ch_h), (8000.0, 2000.0), "the group's member space");
        assert_eq!(children.len(), 2, "both boxes, with box B {height_b} high");
        assert_eq!((children[0].x, children[1].x), (100.0, 4200.0), "side by side");

        let stories: Vec<u32> = children.iter().filter_map(|c| c.obj.text_box()).collect();
        assert_eq!(stories.len(), 2, "each box has its own story");
        assert_ne!(stories[0], stories[1]);
        assert_eq!(part_text(&d, stories[0]), "Box A (tall) line 1\nBox A (tall) line 2\nBox A (tall) line 3");
        assert_eq!(part_text(&d, stories[1]), format!("{label_b} line 1\n{label_b} line 2\n{label_b} line 3"));

        // Saved and read again -- the step the report loses the text at.
        let back = wordcraft_docx::read(&wordcraft_docx::write(&d).unwrap()).unwrap();
        let InlineObject::Group { children: again, .. } = &objects(&back)[0] else { panic!() };
        let back_stories: Vec<u32> = again.iter().filter_map(|c| c.obj.text_box()).collect();
        assert_eq!(back_stories.len(), 2, "both boxes survive the save");
        assert_eq!(part_text(&back, back_stories[0]), part_text(&d, stories[0]));
        assert_eq!(part_text(&back, back_stories[1]), part_text(&d, stories[1]));
    }
}

#[test]
fn a_group_member_finds_its_shapetype_beside_the_group() {
    // Word writes each `v:shapetype` once, at its first use, as a child of the `w:pict` -- so a
    // group's member refers to one that is its *parent's* sibling, not its own. The member here
    // carries no geometry and no colours of its own: its diamond shape comes from the
    // shapetype's `o:spt` (4) and its fill from the shapetype's `fillcolor`.
    let d = read_body(&pict(concat!(
        r##"<v:shapetype id="_x0000_t4" o:spt="4" fillcolor="#0000ff"/>"##,
        r#"<v:group style="width:100pt;height:50pt" coordsize="1000,500">"#,
        r##"<v:shape style="left:0;top:0;width:400;height:500" type="#_x0000_t4"/>"##,
        r#"</v:group>"#,
    )));
    let InlineObject::Group { children, .. } = &objects(&d)[0] else { panic!("{:?}", objects(&d)) };
    assert_eq!(children.len(), 1, "the member is read");
    let InlineObject::Shape { kind, fill, .. } = children[0].obj else { panic!("{:?}", children[0].obj) };
    assert_eq!(kind, ShapeKind::Diamond, "the preset comes from the shapetype's o:spt");
    assert_eq!(fill, Some(wordcraft_doc::Rgb(0x00, 0x00, 0xff)), "and the fill from its fillcolor");
}

#[test]
fn a_top_level_shape_still_finds_its_shapetype() {
    // The same lookup for a shape that is not in a group, which is what it was written for.
    let d = read_body(&pict(concat!(
        r##"<v:shapetype id="_x0000_t4" o:spt="4" fillcolor="#00ff00"/>"##,
        r##"<v:shape style="width:40pt;height:40pt" type="#_x0000_t4"/>"##,
    )));
    let InlineObject::Shape { kind, fill, .. } = objects(&d)[0] else { panic!("{:?}", objects(&d)) };
    assert_eq!((kind, fill), (ShapeKind::Diamond, Some(wordcraft_doc::Rgb(0x00, 0xff, 0x00))));
}
