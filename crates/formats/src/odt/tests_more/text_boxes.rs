//! Text boxes (ODF 1.3 §10.4.3 `draw:frame` holding a `draw:text-box`): their text opens as a
//! text box's story, in place and floating as the frame says, and is written back the same way.

use crate::odt::tests::{NS_T, package};
use crate::odt::{export, import};
use wordcraft_doc::para::{InlineObject, OBJ, ShapeKind, Wrap};
use wordcraft_doc::{Document, PartKind};

const NS_D: &str =
    r#"xmlns:draw="urn:oasis:names:tc:opendocument:xmlns:drawing:1.0" xmlns:svg="urn:oasis:names:tc:opendocument:xmlns:svg-compatible:1.0""#;

fn open(frame_attrs: &str, inner: &str) -> Document {
    let content = format!(
        r#"<office:document-content {NS_T} {NS_D}><office:automatic-styles><style:style style:name="fr1" style:family="graphic"><style:graphic-properties style:wrap="parallel" style:horizontal-pos="right" style:horizontal-rel="paragraph"/></style:style></office:automatic-styles><office:body><office:text><text:p>Before<draw:frame draw:style-name="fr1" svg:width="2in" svg:height="1in" {frame_attrs}><draw:text-box>{inner}</draw:text-box></draw:frame> after.</text:p></office:text></office:body></office:document-content>"#
    );
    import(&package(&content, None)).unwrap()
}

/// Each text box: (its size, floats, its text).
fn boxes(d: &Document) -> Vec<((f32, f32), bool, String)> {
    let mut out = Vec::new();
    for b in d.body.iter() {
        let Some(p) = b.as_para() else { continue };
        for o in &p.objects {
            if let InlineObject::Shape { kind: ShapeKind::TextBox, w, h, float, story: Some(id), .. } = o {
                let part = d.parts.get(id).expect("the box's story");
                assert_eq!(part.kind, PartKind::TextBox);
                let text = part.blocks.iter().filter_map(|b| b.as_para()).map(|p| p.text.replace(OBJ, "")).collect::<Vec<_>>().join("|");
                out.push(((*w, *h), float.wrap != Wrap::Inline, text));
            }
        }
    }
    out
}

#[test]
fn a_text_box_keeps_its_text() {
    let d = open(r#"text:anchor-type="paragraph""#, "<text:p>Inside</text:p><text:p>the box</text:p>");
    assert_eq!(boxes(&d), [((144.0, 72.0), true, "Inside|the box".to_string())]);
    // Its text isn't the paragraph's.
    assert_eq!(d.body[0].as_para().unwrap().text.replace(OBJ, "#"), "Before# after.");
}

#[test]
fn a_box_in_line_and_a_picture_inside_it() {
    let d = open(r#"text:anchor-type="as-char""#, "<text:p>Inline box</text:p>");
    assert_eq!(boxes(&d), [((144.0, 72.0), false, "Inline box".to_string())]);
}

#[test]
fn text_boxes_round_trip() {
    let d = open(r#"text:anchor-type="paragraph""#, "<text:p>Inside</text:p><text:p>the box</text:p>");
    let back = import(&export(&d).unwrap()).unwrap();
    assert_eq!(boxes(&back), boxes(&d));
    assert_eq!(back.body[0].as_para().unwrap().text, d.body[0].as_para().unwrap().text);
}

#[test]
fn hostile_text_boxes_do_not_panic() {
    let deep = r#"<draw:frame svg:width="1in" svg:height="1in"><draw:text-box><text:p>"#.repeat(100);
    for inner in [deep.as_str(), "<text:p>unclosed", "<text:p>a<text:note><text:note-body><text:p>n</text:p></text:note-body></text:note></text:p>"] {
        let d = open(r#"text:anchor-type="paragraph""#, inner);
        let _ = export(&d).unwrap();
    }
}

/// Each text box's look: (fill, outline colour and width, inner margins in points, grows to fit).
#[allow(clippy::type_complexity)]
fn looks(d: &Document) -> Vec<(Option<wordcraft_doc::props::Rgb>, Option<(wordcraft_doc::props::Rgb, f32)>, [f32; 4], bool)> {
    let mut out = Vec::new();
    for b in d.body.iter() {
        let Some(p) = b.as_para() else { continue };
        for o in &p.objects {
            if let InlineObject::Shape { kind: ShapeKind::TextBox, fill, stroke, stroke_width, story: Some(id), .. } = o {
                let body = &d.parts.get(id).expect("the box's story").body;
                let inset = body.insets_pt().map(|v| (v * 100.0).round() / 100.0);
                out.push((fill.as_ref().map(|f| f.main_color()), stroke.map(|c| (c, *stroke_width)), inset, body.auto_fit));
            }
        }
    }
    out
}

/// Two boxes: one in LibreOffice's Frame style (hairline, no fill), one with its own look and
/// growing to its text (no `svg:height`, a `fo:min-height`).
fn open_styled() -> Document {
    let styles = format!(
        r##"<office:document-styles {NS_T} {NS_D}><office:styles><style:style style:name="Frame" style:family="graphic"><style:graphic-properties style:wrap="parallel" draw:fill="none" fo:padding="0.0591in" fo:border="0.06pt solid #000000"/></style:style></office:styles></office:document-styles>"##
    );
    let content = format!(
        r##"<office:document-content {NS_T} {NS_D}><office:automatic-styles><style:style style:name="fr1" style:family="graphic" style:parent-style-name="Frame"><style:graphic-properties style:horizontal-pos="left"/></style:style><style:style style:name="fr2" style:family="graphic" style:parent-style-name="Frame"><style:graphic-properties fo:border="2pt solid #0000ff" fo:padding="3pt" fo:padding-left="10pt" draw:fill="solid" draw:fill-color="#ffff00"/></style:style></office:automatic-styles><office:body><office:text><text:p>A<draw:frame draw:style-name="fr1" text:anchor-type="paragraph" svg:width="2in" svg:height="1in"><draw:text-box><text:p>One</text:p></draw:text-box></draw:frame> b<draw:frame draw:style-name="fr2" text:anchor-type="paragraph" svg:width="2in"><draw:text-box fo:min-height="0.5in"><text:p>Two</text:p></draw:text-box></draw:frame></text:p></office:text></office:body></office:document-content>"##
    );
    import(&package(&content, Some(&styles))).unwrap()
}

#[test]
fn a_box_keeps_its_frame_look() {
    use wordcraft_doc::props::Rgb;
    let d = open_styled();
    assert_eq!(
        looks(&d),
        [
            (None, Some((Rgb(0, 0, 0), 0.06)), [4.26, 4.26, 4.26, 4.26], false),
            (Some(Rgb(255, 255, 0)), Some((Rgb(0, 0, 255), 2.0)), [10.0, 3.0, 3.0, 3.0], true),
        ]
    );
    // A box growing to its text is at least as tall as its minimum.
    assert_eq!(boxes(&d)[1].0, (144.0, 36.0));
}

#[test]
fn a_box_look_round_trips() {
    let d = open_styled();
    let back = import(&export(&d).unwrap()).unwrap();
    assert_eq!(looks(&back), looks(&d));
    assert_eq!(boxes(&back), boxes(&d));
}

#[test]
fn a_box_without_a_style_has_no_line_or_fill() {
    let content = format!(
        r#"<office:document-content {NS_T} {NS_D}><office:body><office:text><text:p><draw:frame text:anchor-type="as-char" svg:width="1in" svg:height="1in"><draw:text-box><text:p>x</text:p></draw:text-box></draw:frame></text:p></office:text></office:body></office:document-content>"#
    );
    let d = import(&package(&content, None)).unwrap();
    assert_eq!(looks(&d).first().map(|l| (l.0, l.1, l.2)), Some((None, None, [0.0; 4])), "ODF's defaults: no line, fill or padding");
}

#[test]
fn a_box_growing_from_nothing_stays_that_way() {
    // LibreOffice's own: no height, a least height of zero.
    let d = open(r#"text:anchor-type="paragraph""#, "<text:p>x</text:p>");
    assert!(!looks(&d)[0].3);
    let content = format!(
        r#"<office:document-content {NS_T} {NS_D}><office:body><office:text><text:p><draw:frame text:anchor-type="paragraph" svg:width="1in"><draw:text-box fo:min-height="0in"><text:p>x</text:p></draw:text-box></draw:frame></text:p></office:text></office:body></office:document-content>"#
    );
    let d = import(&package(&content, None)).unwrap();
    assert!(looks(&d)[0].3);
    assert_eq!(boxes(&d)[0].0, (72.0, 1.0));
    let saved = crate::odt::tests::content_of(&crate::odt::tests::export_model(&d).unwrap());
    assert!(saved.contains(r#"<draw:text-box fo:min-height="1pt">"#) && !saved.contains("svg:height"), "{saved}");
}

#[test]
fn a_text_box_frame_derives_from_the_frame_style() {
    // LibreOffice draws a frame's border (`fo:border`) only when its style derives from the
    // Frame style; otherwise it outlines the box in its default shape line instead.
    let d = open(r#"text:anchor-type="paragraph" svg:width="2in""#, "<text:p>In the box.</text:p>");
    let pkg = crate::odt::tests::export_model(&d).unwrap();
    let content = crate::odt::tests::content_of(&pkg);
    let style = content.split("<style:style ").find(|s| s.contains("fo:border")).unwrap_or_default();
    assert!(style.contains(r#"style:parent-style-name="Frame""#), "{content}");
    assert!(crate::odt::tests::styles_of(&pkg).contains(r#"<style:style style:name="Frame" style:family="graphic"/>"#));
}

/// Each text box's story id and the box its text continues in.
fn links(d: &Document) -> Vec<(u32, Option<u32>)> {
    d.body
        .iter()
        .filter_map(|b| b.as_para())
        .flat_map(|p| p.objects.iter())
        .filter_map(
            |o| if let InlineObject::Shape { story: Some(id), .. } = o { Some((*id, d.parts.get(id).and_then(|p| p.body.next))) } else { None },
        )
        .collect()
}

fn linked(frames: &str) -> Document {
    let content = format!(
        r#"<office:document-content {NS_T} {NS_D}><office:body><office:text><text:p>{frames}</text:p></office:text></office:body></office:document-content>"#
    );
    import(&package(&content, None)).unwrap()
}

#[test]
fn linked_text_boxes_stay_linked() {
    // `draw:chain-next-name` names the frame the text continues in.
    let d = linked(
        r#"<draw:frame draw:name="A" text:anchor-type="paragraph" svg:width="1in" svg:height="1in"><draw:text-box draw:chain-next-name="B"><text:p>Text that flows on.</text:p></draw:text-box></draw:frame><draw:frame draw:name="B" text:anchor-type="paragraph" svg:x="2in" svg:width="1in" svg:height="1in"><draw:text-box/></draw:frame>"#,
    );
    let l = links(&d);
    assert_eq!(l.len(), 2);
    assert_eq!(l[0].1, Some(l[1].0), "{l:?}");
    let saved = crate::odt::tests::export_model(&d).unwrap();
    let c = crate::odt::tests::content_of(&saved);
    assert!(c.contains(&format!(r#"draw:chain-next-name="Frame{}""#, l[1].0)), "{c}");
    let back = links(&import(&saved).unwrap());
    assert_eq!(back[0].1, Some(back[1].0));
}

#[test]
fn a_chain_to_nothing_itself_or_round_is_not_linked() {
    let d = linked(
        r#"<draw:frame draw:name="A" text:anchor-type="paragraph" svg:width="1in" svg:height="1in"><draw:text-box draw:chain-next-name="A"><text:p>x</text:p></draw:text-box></draw:frame><draw:frame draw:name="B" text:anchor-type="paragraph" svg:width="1in" svg:height="1in"><draw:text-box draw:chain-next-name="missing"/></draw:frame><draw:frame draw:name="C" text:anchor-type="paragraph" svg:width="1in" svg:height="1in"><draw:text-box draw:chain-next-name="D"/></draw:frame><draw:frame draw:name="D" text:anchor-type="paragraph" svg:width="1in" svg:height="1in"><draw:text-box draw:chain-next-name="C"/></draw:frame>"#,
    );
    let l = links(&d);
    assert_eq!((l[0].1, l[1].1), (None, None));
    // C → D → C: one of the two links is undone.
    assert!(l[2].1.is_none() || l[3].1.is_none(), "{l:?}");
}

#[test]
fn a_picture_in_a_section_in_a_page_anchored_text_box_is_kept() {
    // LibreOffice's fax templates: page-anchored text boxes before the first paragraph, the
    // last holding the company logo in a section.
    let png = [b"\x89PNG\r\n\x1a\n".as_slice(), &[7u8; 300]].concat();
    let boxes = r#"<draw:frame text:anchor-type="page" text:anchor-page-number="1" svg:x="1in" svg:y="2in" svg:width="4in"><draw:text-box fo:min-height="0.4in"><text:p/><text:p>Important!</text:p></draw:text-box></draw:frame><draw:frame text:anchor-type="page" text:anchor-page-number="1" svg:x="1in" svg:y="0.8in" svg:width="1.5in"><draw:text-box fo:min-height="1.2in"><text:p/><text:section text:name="Company Logo"><text:p><draw:frame draw:style-name="fr4" text:anchor-type="paragraph" svg:width="1in" svg:height="0.7in"><draw:image xlink:href="Pictures/logo.png"/></draw:frame></text:p></text:section></draw:text-box></draw:frame>"#;
    let content = format!(
        r#"<office:document-content {NS_T} {NS_D} xmlns:xlink="http://www.w3.org/1999/xlink"><office:automatic-styles><style:style style:name="fr4" style:family="graphic"><style:graphic-properties style:wrap="parallel" style:horizontal-pos="center" style:horizontal-rel="paragraph"/></style:style></office:automatic-styles><office:body><office:text>{boxes}<text:p>Fax.</text:p></office:text></office:body></office:document-content>"#
    );
    let pkg = crate::odt::tests::zip_of(&[("content.xml", content.as_bytes()), ("Pictures/logo.png", &png)]);
    let d = import(&pkg).unwrap();
    let pictures: usize = d
        .parts
        .values()
        .flat_map(|p| p.blocks.iter())
        .filter_map(|b| b.as_para())
        .flat_map(|p| p.objects.iter())
        .filter(|o| matches!(o, InlineObject::Image { .. }))
        .count();
    assert_eq!(pictures, 1, "the logo in its box");
    assert_eq!(boxes_of(&d), 2);
    // And saved in it: as read, and from the model.
    let c = crate::odt::tests::content_of(&export(&d).unwrap());
    assert_eq!(c.matches("<draw:image ").count(), 1, "{c}");
    let c = crate::odt::tests::content_of(&crate::odt::tests::export_model(&d).unwrap());
    assert_eq!(c.matches("<draw:image ").count(), 1, "{c}");
    // Its frame's style derives from Graphics: LibreOffice shows a floating picture in a text box
    // only so.
    assert!(c.contains(r#"style:family="graphic" style:parent-style-name="Graphics""#), "{c}");
}

/// A tiny placeable Windows metafile, 1 in × 0.5 in: its header and its end record.
fn wmf() -> Vec<u8> {
    let mut v = Vec::new();
    v.extend_from_slice(&[0xd7, 0xcd, 0xc6, 0x9a, 0, 0]);
    for n in [0i16, 0, 1440, 720] {
        v.extend_from_slice(&n.to_le_bytes());
    }
    v.extend_from_slice(&1440u16.to_le_bytes());
    v.extend_from_slice(&[0, 0, 0, 0]);
    let check = v.chunks(2).take(10).fold(0u16, |a, c| a ^ u16::from_le_bytes([c[0], c[1]]));
    v.extend_from_slice(&check.to_le_bytes());
    // The standard header: a memory metafile, 9 words, version 3, its size, no objects.
    for w in [1u16, 9, 0x300] {
        v.extend_from_slice(&w.to_le_bytes());
    }
    v.extend_from_slice(&12u32.to_le_bytes());
    v.extend_from_slice(&0u16.to_le_bytes());
    v.extend_from_slice(&3u32.to_le_bytes());
    v.extend_from_slice(&0u16.to_le_bytes());
    // The end record.
    v.extend_from_slice(&3u32.to_le_bytes());
    v.extend_from_slice(&0u16.to_le_bytes());
    v
}

#[test]
fn a_windows_metafile_picture_is_kept() {
    // LibreOffice keeps clip art and logos as WMF or EMF: they were dropped.
    let content = format!(
        r#"<office:document-content {NS_T} {NS_D} xmlns:xlink="http://www.w3.org/1999/xlink"><office:body><office:text><text:p><draw:frame text:anchor-type="as-char"><draw:image xlink:href="Pictures/logo.wmf"/></draw:frame> and <draw:frame text:anchor-type="as-char" svg:width="2in" svg:height="1in"><draw:image xlink:href="Pictures/logo.wmf"/></draw:frame></text:p></office:text></office:body></office:document-content>"#
    );
    let pkg = crate::odt::tests::zip_of(&[("content.xml", content.as_bytes()), ("Pictures/logo.wmf", &wmf())]);
    let d = import(&pkg).unwrap();
    let sizes: Vec<(f32, f32)> = d.body[0]
        .as_para()
        .unwrap()
        .objects
        .iter()
        .filter_map(|o| if let InlineObject::Image { w, h, .. } = o { Some((*w, *h)) } else { None })
        .collect();
    // Its own size (1 in × 0.5 in) when the frame gives none, else the frame's.
    assert_eq!(sizes, [(72.0, 36.0), (144.0, 72.0)]);
    // Saved with its own media type.
    let saved = export(&d).unwrap();
    assert!(crate::odt::tests::entry(&saved, "META-INF/manifest.xml").contains("image/x-wmf"));
    assert_eq!(import(&saved).unwrap().body[0].as_para().unwrap().objects.len(), 2);
}

fn boxes_of(d: &Document) -> usize {
    d.body
        .iter()
        .filter_map(|b| b.as_para())
        .flat_map(|p| p.objects.iter())
        .filter(|o| matches!(o, InlineObject::Shape { kind: ShapeKind::TextBox, .. }))
        .count()
}

#[test]
fn a_custom_style_only_in_a_text_box_is_kept() {
    // LibreOffice's fax template: "Frame contents" (centred) names only paragraphs in a text box.
    let content = format!(
        r#"<office:document-content {NS_T} {NS_D}><office:body><office:text><text:p>Before<draw:frame svg:width="2in" svg:height="1in" text:anchor-type="paragraph"><draw:text-box><text:p text:style-name="Frame_20_contents">Important!</text:p></draw:text-box></draw:frame></text:p></office:text></office:body></office:document-content>"#
    );
    let styles = format!(
        r#"<office:document-styles {NS_T}><office:styles><style:style style:name="Standard" style:family="paragraph"/><style:style style:name="Frame_20_contents" style:display-name="Frame contents" style:family="paragraph" style:parent-style-name="Standard"><style:paragraph-properties fo:text-align="center"/></style:style></office:styles></office:document-styles>"#
    );
    let centred = |d: &Document| {
        let part = d.parts.values().find(|p| p.kind == PartKind::TextBox).expect("the box's story");
        let p = part.blocks[0].as_para().unwrap();
        let style = p.props.style.clone().expect("a style");
        assert!(d.styles.get(&style).is_some(), "{style} is defined");
        d.styles.resolve_para(&p.props).align
    };
    let d = import(&package(&content, Some(&styles))).unwrap();
    assert_eq!(centred(&d), wordcraft_doc::props::Align::Center);
    let back = import(&export(&d).unwrap()).unwrap();
    assert_eq!(centred(&back), wordcraft_doc::props::Align::Center);
}

#[test]
fn a_frames_drawn_stroke_is_its_line_when_it_has_no_border() {
    // LibreOffice's letter logo box: `draw:stroke="solid"` with a 0 width (a hairline) and no
    // `fo:border`; an explicit "none" border still has no line.
    let open_with = |props: &str| {
        let content = format!(
            r#"<office:document-content {NS_T} {NS_D}><office:automatic-styles><style:style style:name="g" style:family="graphic"><style:graphic-properties {props}/></style:style></office:automatic-styles><office:body><office:text><text:p><draw:frame draw:style-name="g" svg:width="2in" svg:height="1in" text:anchor-type="paragraph"><draw:text-box><text:p>Logo</text:p></draw:text-box></draw:frame></text:p></office:text></office:body></office:document-content>"#
        );
        import(&package(&content, None)).unwrap()
    };
    let line = |d: &Document| looks(d).first().map(|l| l.1).expect("a box");
    let stroke = r##"draw:stroke="solid" svg:stroke-width="0cm" svg:stroke-color="#000000""##;
    // A 0 width draws at a hairline, 0.25 pt.
    assert_eq!(line(&open_with(stroke)), Some((wordcraft_doc::props::Rgb::BLACK, 0.25)));
    assert_eq!(line(&open_with(&format!(r#"{stroke} fo:border="none""#))), None);
    assert_eq!(line(&open_with("")), None);
}
