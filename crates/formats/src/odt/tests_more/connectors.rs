//! Connectors (ODF 1.3 §10.3.10 `draw:connector`): lines glued to other shapes' glue points
//! (§9.2.19), read as WordCraft connectors glued to the shapes' connection sites, placed between
//! them, and written back glued.

use crate::odt::import;
use crate::odt::tests::{NS_T, content_of, package};
use wordcraft_doc::para::{InlineObject, ShapeKind};

const NS_D: &str =
    r#"xmlns:draw="urn:oasis:names:tc:opendocument:xmlns:drawing:1.0" xmlns:svg="urn:oasis:names:tc:opendocument:xmlns:svg-compatible:1.0""#;

/// A rectangle (`id1`) and an ellipse (`id2`) anchored to one paragraph, and a connector of
/// `kind` from the rectangle's bottom (glue point 2) to the ellipse's top (0), its own
/// coordinates in LibreOffice's drawing layer (far from the shapes).
fn doc(kind: &str) -> wordcraft_doc::Document {
    let content = format!(
        r#"<office:document-content {NS_T} {NS_D}><office:body><office:text><text:p><draw:rect text:anchor-type="paragraph" xml:id="id1" draw:id="id1" svg:width="1in" svg:height="0.5in" svg:x="0in" svg:y="0in"/><draw:ellipse text:anchor-type="paragraph" xml:id="id2" draw:id="id2" svg:width="1in" svg:height="0.5in" svg:x="3in" svg:y="1in"/><draw:connector text:anchor-type="paragraph"{kind} svg:x1="2in" svg:y1="4in" svg:x2="5in" svg:y2="5in" draw:start-shape="id1" draw:start-glue-point="2" draw:end-shape="id2" draw:end-glue-point="0"/>Text.</text:p></office:text></office:body></office:document-content>"#
    );
    import(&package(&content, None)).unwrap()
}

/// The paragraph's shapes: (kind, frame x, y, w, h, flips, drawing id, glued ends).
#[allow(clippy::type_complexity)]
fn shapes(d: &wordcraft_doc::Document) -> Vec<(ShapeKind, [f32; 4], (bool, bool), u32, Option<(u32, u32)>, Option<(u32, u32)>)> {
    d.body
        .iter()
        .filter_map(|b| b.as_para())
        .flat_map(|p| p.objects.iter())
        .filter_map(|o| match o {
            InlineObject::Shape { kind, w, h, float, extra, .. } => Some((
                *kind,
                [float.x, float.y, *w, *h].map(|v| (v * 10.0).round() / 10.0),
                (float.flip_h, float.flip_v),
                extra.id,
                extra.start.map(|e| (e.id, e.site)),
                extra.end.map(|e| (e.id, e.site)),
            )),
            _ => None,
        })
        .collect()
}

#[test]
fn a_connector_is_glued_between_its_shapes() {
    let s = shapes(&doc(""));
    assert_eq!(s.len(), 3);
    assert_eq!((s[0].0, s[0].3), (ShapeKind::Rectangle, 1));
    assert_eq!((s[1].0, s[1].3), (ShapeKind::Ellipse, 2));
    // From the rectangle's bottom (its site 2) to the ellipse's top (site 0), and placed so:
    // (36, 36) to (252, 72).
    assert_eq!(s[2], (ShapeKind::ElbowConnector, [36.0, 36.0, 216.0, 36.0], (false, false), 0, Some((1, 2)), Some((2, 0))));
    // Its type: straight and curved.
    assert_eq!(shapes(&doc(r#" draw:type="line""#))[2].0, ShapeKind::StraightConnector);
    assert_eq!(shapes(&doc(r#" draw:type="curve""#))[2].0, ShapeKind::CurvedConnector);
}

#[test]
fn connectors_are_written_glued_and_read_back() {
    let d = doc("");
    let c = content_of(&crate::odt::tests::export_model(&d).unwrap());
    assert!(c.contains(r#"<draw:rect"#) && c.contains(r#"xml:id="shape1" draw:id="shape1""#), "{c}");
    assert!(
        c.contains(r#"draw:type="standard" draw:start-shape="shape1" draw:start-glue-point="2" draw:end-shape="shape2" draw:end-glue-point="0""#),
        "{c}"
    );
    assert_eq!(shapes(&import(&crate::odt::tests::export_model(&d).unwrap()).unwrap()), shapes(&d));
}

#[test]
fn glue_to_nothing_or_a_glue_point_of_its_own_is_dropped() {
    let content = format!(
        r#"<office:document-content {NS_T} {NS_D}><office:body><office:text><text:p><draw:rect text:anchor-type="paragraph" xml:id="a" svg:width="1in" svg:height="1in" svg:x="0in" svg:y="0in"/><draw:connector text:anchor-type="paragraph" svg:x1="0in" svg:y1="0in" svg:x2="1in" svg:y2="1in" draw:start-shape="a" draw:start-glue-point="7" draw:end-shape="missing" draw:end-glue-point="1"/><draw:connector text:anchor-type="paragraph" svg:x1="0in" svg:y1="0in" svg:x2="1in" svg:y2="1in" draw:start-shape="a" draw:start-glue-point="99999999999"/></text:p></office:text></office:body></office:document-content>"#
    );
    let s = shapes(&import(&package(&content, None)).unwrap());
    assert_eq!((s[1].4, s[1].5), (None, None));
    assert_eq!(s[2].4, None);
}

#[test]
fn a_connector_in_a_group_is_glued_by_its_shapes_sites() {
    // In a group, a connector glued to the group's ellipse: its glue point is the ellipse's site
    // (right: glue point 1 is the ellipse's site 6), not taken as a site number itself.
    let content = format!(
        r#"<office:document-content {NS_T} {NS_D}><office:body><office:text><text:p><draw:g text:anchor-type="paragraph"><draw:ellipse xml:id="e" svg:width="1in" svg:height="1in" svg:x="0in" svg:y="0in"/><draw:rect xml:id="r" svg:width="1in" svg:height="1in" svg:x="2in" svg:y="0in"/><draw:connector svg:x1="1in" svg:y1="0.5in" svg:x2="2in" svg:y2="0.5in" draw:start-shape="e" draw:start-glue-point="1" draw:end-shape="r" draw:end-glue-point="3"/></draw:g></text:p></office:text></office:body></office:document-content>"#
    );
    let d = import(&package(&content, None)).unwrap();
    let ends: Vec<(Option<(u32, u32)>, Option<(u32, u32)>)> = d
        .body
        .iter()
        .filter_map(|b| b.as_para())
        .flat_map(|p| p.objects.iter())
        .filter_map(|o| if let InlineObject::Group { children, .. } = o { Some(children) } else { None })
        .flatten()
        .filter_map(|c| match &c.obj {
            InlineObject::Shape { kind, extra, .. } if kind.is_connector() => {
                Some((extra.start.map(|e| (e.id, e.site)), extra.end.map(|e| (e.id, e.site))))
            }
            _ => None,
        })
        .collect();
    assert_eq!(ends, [(Some((1, 6)), Some((2, 1)))]);
}
