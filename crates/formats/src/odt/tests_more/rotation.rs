//! Rotated and mirrored shapes and pictures (ODF 1.3 §19.228 `draw:transform`, §20.112
//! `draw:mirror-horizontal`/`-vertical` of a custom shape's geometry): a turn in the file
//! (anticlockwise, about the shape's top left corner, then moved) opens as WordCraft's
//! (clockwise about the centre of the unrotated box at its offset), and is written back.

use crate::odt::tests::{NS_T, content_of, package, zip_of};
use crate::odt::{export, import};
use wordcraft_doc::Document;
use wordcraft_doc::para::{Float, InlineObject, Wrap};

const NS_D: &str = r#"xmlns:draw="urn:oasis:names:tc:opendocument:xmlns:drawing:1.0" xmlns:svg="urn:oasis:names:tc:opendocument:xmlns:svg-compatible:1.0" xmlns:xlink="http://www.w3.org/1999/xlink""#;

/// Each object's (offset, rotation, flips, floats), rounded.
fn placed(d: &Document) -> Vec<((f32, f32), f32, (bool, bool), bool)> {
    let r = |v: f32| (v * 10.0).round() / 10.0;
    d.body
        .iter()
        .filter_map(|b| b.as_para())
        .flat_map(|p| p.objects.iter())
        .filter_map(|o| match o {
            InlineObject::Shape { float, .. } | InlineObject::Image { float, .. } => Some(float),
            _ => None,
        })
        .map(|f: &Float| ((r(f.x), r(f.y)), r(f.rot), (f.flip_h, f.flip_v), f.wrap != Wrap::Inline))
        .collect()
}

/// LibreOffice's own: a rectangle turned 30° and an arrow turned 90°, both anticlockwise, whose
/// unrotated boxes sit at (0.3937in, 0.1969in) and (2.756in, 0.1969in); an arrow mirrored.
fn open() -> Document {
    let content = format!(
        r#"<office:document-content {NS_T} {NS_D}><office:body><office:text><text:p>
<draw:custom-shape text:anchor-type="paragraph" svg:width="1.5752in" svg:height="0.5906in" draw:transform="rotate (0.523598775598299) translate (0.351388888888889in 0.629861111111111in)"><draw:enhanced-geometry draw:type="rectangle"/></draw:custom-shape>
<draw:custom-shape text:anchor-type="paragraph" svg:width="1.1815in" svg:height="0.5906in" draw:transform="rotate (1.5707963267949) translate (3.05138888888889in 1.08194444444444in)"><draw:enhanced-geometry draw:type="right-arrow"/></draw:custom-shape>
<draw:custom-shape text:anchor-type="paragraph" svg:width="1in" svg:height="0.5in" svg:x="1in" svg:y="2in"><draw:enhanced-geometry draw:type="right-arrow" draw:mirror-horizontal="true"/></draw:custom-shape>
<draw:custom-shape text:anchor-type="as-char" svg:width="1in" svg:height="0.5in" draw:transform="rotate (-0.785398163397448) translate (0in 0in)"><draw:enhanced-geometry draw:type="diamond" draw:mirror-vertical="true"/></draw:custom-shape>
text</text:p></office:text></office:body></office:document-content>"#
    );
    import(&package(&content, None)).unwrap()
}

#[test]
fn turned_shapes_open_turned_about_their_centre() {
    assert_eq!(
        placed(&open()),
        [
            ((28.3, 14.2), 330.0, (false, false), true),
            ((198.4, 14.1), 270.0, (false, false), true),
            ((72.0, 144.0), 0.0, (true, false), true),
            // In line with the text: turned 45° clockwise, mirrored, still in line.
            ((0.0, 0.0), 45.0, (false, true), false),
        ]
    );
}

#[test]
fn turned_shapes_round_trip() {
    let d = open();
    let bytes = export(&d).unwrap();
    let c = content_of(&bytes);
    assert!(c.contains("draw:transform=\"rotate (") && c.contains("draw:mirror-horizontal=\"true\""), "{c}");
    assert_eq!(placed(&import(&bytes).unwrap()), placed(&d));
}

#[test]
fn a_turned_picture_opens_and_saves_turned() {
    let png = [b"\x89PNG\r\n\x1a\n".as_slice(), &[7u8; 64]].concat();
    let content = format!(
        r#"<office:document-content {NS_T} {NS_D}><office:body><office:text><text:p><draw:frame text:anchor-type="paragraph" svg:width="2in" svg:height="1in" draw:transform="rotate (1.5707963267949) translate (1in 3in)"><draw:image xlink:href="Pictures/a.png"/></draw:frame>Text.</text:p></office:text></office:body></office:document-content>"#
    );
    let d = import(&zip_of(&[("content.xml", content.as_bytes()), ("Pictures/a.png", &png)])).unwrap();
    // Centre: (1in, 3in) + the half size turned a quarter anticlockwise, (0.5in, -1in): the
    // unrotated box's top left is half its size back from (1.5in, 2in).
    assert_eq!(placed(&d), [((36.0, 108.0), 270.0, (false, false), true)]);
    assert_eq!(placed(&import(&export(&d).unwrap()).unwrap()), placed(&d));
}

#[test]
fn hostile_transforms_do_not_panic() {
    for t in [
        "rotate (NaN) translate (1e40in 2in)",
        "rotate",
        "rotate (1",
        "translate (1in)",
        "scale (2 2) rotate (1e30) skewX (3)",
        "matrix (1 0 0 1 0 0)",
        "rotate ((((",
        "",
    ] {
        let content = format!(
            r#"<office:document-content {NS_T} {NS_D}><office:body><office:text><text:p><draw:custom-shape text:anchor-type="paragraph" svg:width="1in" svg:height="1in" draw:transform="{t}"><draw:enhanced-geometry draw:type="star5"/></draw:custom-shape></text:p></office:text></office:body></office:document-content>"#
        );
        let d = import(&package(&content, None)).unwrap();
        for ((x, y), rot, ..) in placed(&d) {
            assert!(x.is_finite() && y.is_finite() && rot.is_finite() && (0.0..360.0).contains(&rot), "{t}: {x} {y} {rot}");
        }
        let _ = export(&d).unwrap();
    }
}
