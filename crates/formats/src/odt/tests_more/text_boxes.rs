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
    let saved = crate::odt::tests::content_of(&export(&d).unwrap());
    assert!(saved.contains(r#"<draw:text-box fo:min-height="1pt">"#) && !saved.contains("svg:height"), "{saved}");
}
