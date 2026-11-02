//! Groups of drawing shapes and pictures (ODF 1.3 §10.3.15 `draw:g`): the group is anchored and
//! wraps as one, its members placed by their own positions in it; opened as a WordCraft group
//! and written back as one.

use crate::odt::tests::{NS_T, content_of, package};
use crate::odt::{export, import};
use wordcraft_doc::Document;
use wordcraft_doc::para::{InlineObject, ShapeKind, Wrap};

const NS_D: &str =
    r#"xmlns:draw="urn:oasis:names:tc:opendocument:xmlns:drawing:1.0" xmlns:svg="urn:oasis:names:tc:opendocument:xmlns:svg-compatible:1.0""#;

/// A group as read: (size, floats, offset, its members (kind, offset in the group, size, turn, text)).
#[allow(clippy::type_complexity)]
fn groups(d: &Document) -> Vec<((f32, f32), bool, (f32, f32), Vec<(ShapeKind, (f32, f32), (f32, f32), f32, String)>)> {
    let r = |v: f32| (v * 10.0).round() / 10.0;
    let mut out = Vec::new();
    for b in d.body.iter() {
        let Some(p) = b.as_para() else { continue };
        for o in &p.objects {
            let InlineObject::Group { w, h, float, ch_w, ch_h, children, .. } = o else { continue };
            let (sx, sy) = (w / ch_w, h / ch_h);
            let members = children
                .iter()
                .filter_map(|c| match &c.obj {
                    InlineObject::Shape { kind, w, h, float, story, .. } => {
                        let text = story
                            .and_then(|id| d.parts.get(&id))
                            .map(|p| p.blocks.iter().filter_map(|b| b.as_para()).map(|p| p.text.clone()).collect::<String>())
                            .unwrap_or_default();
                        Some((*kind, (r(c.x * sx), r(c.y * sy)), (r(w * sx), r(h * sy)), r(float.rot), text))
                    }
                    _ => None,
                })
                .collect();
            out.push(((r(*w), r(*h)), float.wrap != Wrap::Inline, (r(float.x), r(float.y)), members));
        }
    }
    out
}

fn open() -> Document {
    let content = format!(
        r#"<office:document-content {NS_T} {NS_D}><office:automatic-styles><style:style style:name="gr1" style:family="graphic"><style:graphic-properties style:wrap="none" style:vertical-pos="from-top" style:vertical-rel="paragraph" style:horizontal-pos="from-left" style:horizontal-rel="paragraph"/></style:style></office:automatic-styles><office:body><office:text>
<text:p>Before<draw:g text:anchor-type="paragraph" draw:style-name="gr1"><draw:custom-shape svg:width="1in" svg:height="1in" svg:x="2in" svg:y="0.5in"><text:p>One</text:p><draw:enhanced-geometry draw:type="ellipse"/></draw:custom-shape><draw:rect svg:width="1in" svg:height="0.5in" svg:x="3in" svg:y="1in"/><draw:g><draw:custom-shape svg:width="0.5in" svg:height="0.5in" draw:transform="rotate (1.5707963267949) translate (2in 2in)"><draw:enhanced-geometry draw:type="star5"/></draw:custom-shape></draw:g></draw:g> after.</text:p>
<text:p>In line <draw:g text:anchor-type="as-char"><draw:rect svg:width="0.5in" svg:height="0.5in" svg:x="0in" svg:y="0in"/><draw:ellipse svg:width="0.5in" svg:height="0.5in" svg:x="1in" svg:y="0in"/></draw:g>.</text:p>
</office:text></office:body></office:document-content>"#
    );
    import(&package(&content, None)).unwrap()
}

#[test]
fn a_group_opens_as_one_object_with_its_members_in_place() {
    let d = open();
    assert_eq!(
        groups(&d),
        [
            (
                // From (2in, 0.5in) to (4in, 2in): the members' boxes (the star's unturned).
                (144.0, 108.0),
                true,
                (144.0, 36.0),
                vec![
                    (ShapeKind::Ellipse, (0.0, 0.0), (72.0, 72.0), 0.0, "One".into()),
                    (ShapeKind::Rectangle, (72.0, 36.0), (72.0, 36.0), 0.0, String::new()),
                    // A group in the group: its members are the group's. The star is turned a
                    // quarter anticlockwise about (2in, 2in): its unturned box is at (2in, 1.5in).
                    (ShapeKind::Star, (0.0, 72.0), (36.0, 36.0), 270.0, String::new()),
                ]
            ),
            (
                (108.0, 36.0),
                false,
                (0.0, 0.0),
                vec![
                    (ShapeKind::Rectangle, (0.0, 0.0), (36.0, 36.0), 0.0, String::new()),
                    (ShapeKind::Ellipse, (72.0, 0.0), (36.0, 36.0), 0.0, String::new())
                ]
            ),
        ]
    );
    // The members' text isn't the paragraph's.
    assert_eq!(d.body[0].as_para().unwrap().text.replace(wordcraft_doc::para::OBJ, "#"), "Before# after.");
}

#[test]
fn groups_round_trip() {
    let d = open();
    let bytes = export(&d).unwrap();
    let c = content_of(&bytes);
    assert_eq!(c.matches("<draw:g ").count(), 2, "{c}");
    assert_eq!(groups(&import(&bytes).unwrap()), groups(&d));
}

#[test]
fn hostile_groups_do_not_panic() {
    let deep = "<draw:g>".repeat(200);
    for inner in [
        format!("{deep}<draw:rect svg:width=\"1in\" svg:height=\"1in\"/>"),
        "<draw:g text:anchor-type=\"paragraph\"></draw:g>".to_string(),
        "<draw:g text:anchor-type=\"paragraph\"><draw:rect svg:width=\"1in\" svg:height=\"1in\" svg:x=\"1e30in\" svg:y=\"-1e30in\"/><draw:rect svg:width=\"1in\" svg:height=\"1in\" svg:x=\"-1e30in\"/>".to_string(),
        format!("<draw:g text:anchor-type=\"paragraph\">{}</draw:g>", "<draw:rect svg:width=\"1in\" svg:height=\"1in\"/>".repeat(1500)),
        "<text:note><text:note-body><text:p><draw:g><draw:rect svg:width=\"1in\" svg:height=\"1in\"/></draw:g></text:p></text:note-body></text:note>".to_string(),
    ] {
        let content = format!(r#"<office:document-content {NS_T} {NS_D}><office:body><office:text><text:p>x{inner}</text:p></office:text></office:body></office:document-content>"#);
        let d = import(&package(&content, None)).unwrap();
        for g in groups(&d) {
            assert!(g.0.0.is_finite() && g.0.1.is_finite() && g.3.len() <= wordcraft_doc::para::MAX_GROUP_CHILDREN, "{g:?}");
        }
        let _ = export(&d).unwrap();
    }
}

#[test]
fn a_diagram_is_saved_as_a_group_of_its_drawing() {
    // A SmartArt diagram (ODF has none): saved as a group of its shapes and its text in text
    // boxes, so every reader draws it; WordCraft opens it back as a group.
    use wordcraft_doc::graphic::{Graphic, GraphicItem, GraphicKind, TextAlign};
    use wordcraft_doc::props::Rgb;
    let blue = Rgb(0x15, 0x60, 0x82);
    let graphic = Graphic {
        kind: GraphicKind::Diagram,
        items: vec![
            GraphicItem::Shape { rect: [0.0, 0.0, 72.0, 36.0], kind: ShapeKind::RoundedRectangle, fill: Some(blue), stroke: None, stroke_width: 0.0 },
            GraphicItem::Text {
                rect: [0.0, 0.0, 72.0, 36.0],
                text: "Plan".into(),
                size: 12.0,
                color: Rgb::WHITE,
                bold: false,
                align: TextAlign::Center,
                font: None,
            },
            GraphicItem::Shape { rect: [90.0, 9.0, 18.0, 18.0], kind: ShapeKind::Arrow, fill: Some(blue), stroke: None, stroke_width: 0.0 },
        ],
        w: 108.0,
        h: 36.0,
        ..Graphic::default()
    };
    let mut d = Document::new();
    let mut p = wordcraft_doc::Paragraph::new();
    let obj = InlineObject::Graphic { w: 216.0, h: 72.0, alt: "Steps".into(), float: Default::default(), graphic: std::sync::Arc::new(graphic) };
    p.insert_object(0, obj, &Default::default()).unwrap();
    d.body = vec![wordcraft_doc::para_block(p)];
    let saved = export(&d).unwrap();
    assert!(content_of(&saved).contains("<draw:g"), "{}", content_of(&saved));
    let g = groups(&import(&saved).unwrap());
    assert_eq!(g.len(), 1);
    let ((w, h), _, _, members) = &g[0];
    assert_eq!((*w, *h), (216.0, 72.0), "drawn at the object's size: twice the graphic's");
    let kinds: Vec<ShapeKind> = members.iter().map(|m| m.0).collect();
    assert!(kinds.contains(&ShapeKind::RoundedRectangle) && kinds.contains(&ShapeKind::Arrow), "{kinds:?}");
    assert!(members.iter().any(|m| m.4 == "Plan"), "its text: {members:?}");
}
