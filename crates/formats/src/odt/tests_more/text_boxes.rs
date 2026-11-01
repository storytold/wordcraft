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
