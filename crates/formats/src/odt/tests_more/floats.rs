//! Floating pictures (ODF 1.3 §9.2 frames anchored to a paragraph, character or page, §20.3xx
//! `style:wrap`, `style:horizontal-pos/-rel`, `style:vertical-pos/-rel`): placed and wrapped as
//! the file says, and written back the same way.

use crate::odt::tests::{NS_T, zip_of};
use crate::odt::{export, import};
use wordcraft_doc::Document;
use wordcraft_doc::para::{Anchor, Float, FloatAlign, InlineObject, Wrap, WrapText};

const NS_D: &str = r#"xmlns:draw="urn:oasis:names:tc:opendocument:xmlns:drawing:1.0" xmlns:svg="urn:oasis:names:tc:opendocument:xmlns:svg-compatible:1.0" xmlns:xlink="http://www.w3.org/1999/xlink""#;

fn open(graphic: &str, frame: &str) -> Document {
    let png = [b"\x89PNG\r\n\x1a\n".as_slice(), &[7u8; 64]].concat();
    let content = format!(
        r#"<office:document-content {NS_T} {NS_D}><office:automatic-styles><style:style style:name="fr1" style:family="graphic"><style:graphic-properties {graphic}/></style:style></office:automatic-styles><office:body><office:text><text:p><draw:frame draw:style-name="fr1" svg:width="2in" svg:height="1in" {frame}><draw:image xlink:href="Pictures/a.png"/></draw:frame>Text beside.</text:p></office:text></office:body></office:document-content>"#
    );
    import(&zip_of(&[("content.xml", content.as_bytes()), ("Pictures/a.png", &png)])).unwrap()
}

fn float(d: &Document) -> Float {
    d.body[0]
        .as_para()
        .unwrap()
        .objects
        .iter()
        .find_map(|o| if let InlineObject::Image { float, .. } = o { Some(float.clone()) } else { None })
        .expect("a picture")
}

#[test]
fn a_paragraph_anchored_frame_floats_with_text_around_it() {
    let f = float(&open(
        r#"style:wrap="parallel" style:horizontal-pos="right" style:horizontal-rel="paragraph" style:vertical-pos="top" style:vertical-rel="paragraph" fo:margin-left="0.1in" fo:margin-right="0.1in" fo:margin-top="0.05in" fo:margin-bottom="0.05in""#,
        r#"text:anchor-type="paragraph""#,
    ));
    assert_eq!((f.wrap, f.wrap_text), (Wrap::Square, WrapText::BothSides));
    assert_eq!((f.h_rel, f.h_align), (Anchor::Paragraph, Some(FloatAlign::End)));
    assert_eq!((f.v_rel, f.v_align), (Anchor::Paragraph, Some(FloatAlign::Start)));
    assert!((f.dist - 7.2).abs() < 0.01 && (f.dist_top - 3.6).abs() < 0.01, "{f:?}");
}

#[test]
fn wrap_modes_and_offsets() {
    let f = float(&open(
        r#"style:wrap="none" style:horizontal-pos="from-left" style:vertical-pos="from-top""#,
        r#"text:anchor-type="paragraph" svg:x="1in" svg:y="0.5in""#,
    ));
    assert_eq!(f.wrap, Wrap::TopAndBottom);
    assert_eq!((f.h_align, f.x, f.v_align, f.y), (None, 72.0, None, 36.0));
    let f = float(&open(r#"style:wrap="run-through" style:run-through="background""#, r#"text:anchor-type="page""#));
    assert_eq!((f.wrap, f.h_rel, f.v_rel), (Wrap::BehindText, Anchor::Page, Anchor::Page));
    assert_eq!(float(&open(r#"style:wrap="run-through""#, r#"text:anchor-type="char""#)).wrap, Wrap::InFrontOfText);
    assert_eq!(float(&open(r#"style:wrap="left""#, r#"text:anchor-type="paragraph""#)).wrap_text, WrapText::Left);
    assert_eq!(float(&open(r#"style:wrap="dynamic""#, r#"text:anchor-type="paragraph""#)).wrap_text, WrapText::Largest);
    // In line with the text.
    assert_eq!(float(&open("", r#"text:anchor-type="as-char""#)).wrap, Wrap::Inline);
}

#[test]
fn floating_pictures_round_trip() {
    for (graphic, frame) in [
        (
            r#"style:wrap="parallel" style:horizontal-pos="right" style:horizontal-rel="paragraph" style:vertical-pos="top" style:vertical-rel="paragraph" fo:margin-left="0.1in" fo:margin-right="0.1in""#,
            r#"text:anchor-type="paragraph""#,
        ),
        (
            r#"style:wrap="none" style:horizontal-pos="from-left" style:horizontal-rel="page" style:vertical-pos="from-top" style:vertical-rel="page""#,
            r#"text:anchor-type="page" svg:x="1in" svg:y="2in""#,
        ),
        (
            r#"style:wrap="run-through" style:run-through="background" style:horizontal-pos="center" style:horizontal-rel="page-content""#,
            r#"text:anchor-type="paragraph""#,
        ),
        ("", r#"text:anchor-type="as-char""#),
    ] {
        let d = open(graphic, frame);
        let back = import(&export(&d).unwrap()).unwrap();
        let (a, b) = (float(&d), float(&back));
        assert_eq!(
            (a.wrap, a.wrap_text, a.h_rel, a.v_rel, a.h_align, a.v_align),
            (b.wrap, b.wrap_text, b.h_rel, b.v_rel, b.h_align, b.v_align),
            "{graphic}"
        );
        assert!((a.x - b.x).abs() < 0.01 && (a.y - b.y).abs() < 0.01 && (a.dist - b.dist).abs() < 0.01, "{graphic}: {a:?} {b:?}");
    }
}

#[test]
fn hostile_frame_positions_are_clamped() {
    let f = float(&open(r#"style:wrap="parallel" fo:margin-left="1e30in""#, r#"text:anchor-type="paragraph" svg:x="-1e30in" svg:y="NaNin""#));
    assert!(f.x.is_finite() && f.x.abs() <= 1584.0 && f.y.is_finite() && f.dist.is_finite() && f.dist <= 1584.0, "{f:?}");
}
