//! Polygons, polylines, paths and regular polygons (ODF 1.3 §10.3.4–10.3.7, §10.3.9:
//! `draw:polygon`, `draw:polyline`, `draw:path` with its SVG path data, `draw:regular-polygon`)
//! and circles (§10.3.8): opened as WordCraft freeforms (curves followed by short straight
//! segments) and ellipses, and freeforms written back as paths.

use crate::odt::tests::{NS_T, content_of, package};
use crate::odt::{export, import};
use wordcraft_doc::Document;
use wordcraft_doc::para::{InlineObject, ShapeKind};

const NS_D: &str =
    r#"xmlns:draw="urn:oasis:names:tc:opendocument:xmlns:drawing:1.0" xmlns:svg="urn:oasis:names:tc:opendocument:xmlns:svg-compatible:1.0""#;

/// Each shape: (kind, frame size, its paths as points in frame points rounded, closed).
#[allow(clippy::type_complexity)]
fn drawn(d: &Document) -> Vec<(ShapeKind, (f32, f32), Vec<(Vec<(f32, f32)>, bool)>)> {
    let r = |v: f32| (v * 10.0).round() / 10.0;
    d.body
        .iter()
        .filter_map(|b| b.as_para())
        .flat_map(|p| p.objects.iter())
        .filter_map(|o| match o {
            InlineObject::Shape { kind, w, h, freeform, .. } => {
                let paths = freeform
                    .as_ref()
                    .map(|f| {
                        f.placed(0.0, 0.0, *w, *h)
                            .into_iter()
                            .map(|(pts, closed)| (pts.iter().map(|[x, y]| (r(*x), r(*y))).collect(), closed))
                            .collect()
                    })
                    .unwrap_or_default();
                Some((*kind, (r(*w), r(*h)), paths))
            }
            _ => None,
        })
        .collect()
}

fn open(shapes: &str) -> Document {
    let content = format!(
        r#"<office:document-content {NS_T} {NS_D}><office:body><office:text><text:p>{shapes}Text.</text:p></office:text></office:body></office:document-content>"#
    );
    import(&package(&content, None)).unwrap()
}

#[test]
fn polygons_and_polylines_open_as_freeforms() {
    let d = open(
        r#"<draw:polygon text:anchor-type="paragraph" svg:width="2in" svg:height="1in" svg:viewBox="100 100 2000 1000" draw:points="100,100 2100,100 1100,1100"/><draw:polyline text:anchor-type="as-char" svg:width="1in" svg:height="1in" svg:viewBox="0 0 10 10" draw:points="0,10 5,0 10,10"/>"#,
    );
    assert_eq!(
        drawn(&d),
        [
            (ShapeKind::Freeform, (144.0, 72.0), vec![(vec![(0.0, 0.0), (144.0, 0.0), (72.0, 72.0)], true)]),
            (ShapeKind::Freeform, (72.0, 72.0), vec![(vec![(0.0, 72.0), (36.0, 0.0), (72.0, 72.0)], false)]),
        ]
    );
}

#[test]
fn a_path_follows_its_svg_data() {
    // Two subpaths: a closed square drawn with absolute, relative, horizontal and vertical
    // moves, and an open cubic curve.
    let d = open(
        r#"<draw:path text:anchor-type="paragraph" svg:width="1in" svg:height="1in" svg:viewBox="0 0 100 100" svg:d="M0 0 H50 v50 l-50 0 Z m60 0 C60 100 100 100 100 0"/>"#,
    );
    let [(kind, size, paths)] = drawn(&d).try_into().unwrap();
    assert_eq!((kind, size), (ShapeKind::Freeform, (72.0, 72.0)));
    assert_eq!(paths.len(), 2);
    assert_eq!(paths[0], (vec![(0.0, 0.0), (36.0, 0.0), (36.0, 36.0), (0.0, 36.0)], true));
    // The curve, in short straight steps, from its start to its end through its middle.
    let (curve, closed) = &paths[1];
    assert!(!closed);
    assert!(curve.len() > 8, "{curve:?}");
    assert_eq!((curve.first(), curve.last()), (Some(&(43.2, 0.0)), Some(&(72.0, 0.0))));
    assert!(curve.iter().any(|(x, y)| (x - 57.6).abs() < 1.0 && (y - 54.0).abs() < 1.0), "the curve's middle: {curve:?}");
}

#[test]
fn arcs_and_quadratic_curves_reach_their_ends() {
    let d =
        open(r#"<draw:path svg:width="1in" svg:height="1in" svg:viewBox="0 0 100 100" svg:d="M0 50 A50 50 0 0 1 100 50 Q100 100 50 100 T0 50 z"/>"#);
    let [(_, _, paths)] = drawn(&d).try_into().unwrap();
    let (pts, closed) = &paths[0];
    assert!(*closed);
    // The half circle's top passes (50, 0); the quadratic pieces end at (50, 100) and (0, 50).
    assert!(pts.iter().any(|(x, y)| (x - 36.0).abs() < 1.0 && y.abs() < 1.0), "{pts:?}");
    assert!(pts.contains(&(72.0, 36.0)) && pts.contains(&(36.0, 72.0)) && pts.last() == Some(&(0.0, 36.0)), "{pts:?}");
}

#[test]
fn regular_polygons_and_circles() {
    let d = open(
        r#"<draw:regular-polygon svg:width="1in" svg:height="1in" draw:corners="5"/><draw:regular-polygon svg:width="1in" svg:height="1in" draw:corners="5" draw:concave="true" draw:sharpness="50%"/><draw:circle svg:width="1in" svg:height="0.5in"/>"#,
    );
    let shapes = drawn(&d);
    assert_eq!(shapes[0].2.len(), 1);
    assert_eq!((shapes[0].2[0].0.len(), shapes[0].2[0].1), (5, true));
    // The first corner at the top middle.
    assert_eq!(shapes[0].2[0].0[0], (36.0, 0.0));
    assert_eq!(shapes[1].2[0].0.len(), 10);
    assert_eq!((shapes[2].0, shapes[2].1), (ShapeKind::Ellipse, (72.0, 36.0)));
}

#[test]
fn freeforms_round_trip_as_paths_or_custom_shapes() {
    let d = open(
        r#"<draw:polygon text:anchor-type="paragraph" svg:width="2in" svg:height="1in" svg:viewBox="0 0 2000 1000" draw:points="0,0 2000,0 1000,1000"/><draw:path svg:width="1in" svg:height="1in" svg:viewBox="0 0 100 100" svg:d="M0 0 H50 v50 l-50 0 Z m60 0 L100 100"/>"#,
    );
    let bytes = crate::odt::tests::export_model(&d).unwrap();
    // The second, with a closed and an open path, as a custom shape (see `custom_geometry`).
    let c = content_of(&bytes);
    assert_eq!((c.matches("<draw:path ").count(), c.matches("<draw:custom-shape ").count()), (1, 1));
    assert_eq!(drawn(&import(&bytes).unwrap()), drawn(&d));
}

#[test]
fn hostile_paths_do_not_panic() {
    let long = "L1 1 ".repeat(20_000);
    for el in [
        format!(r#"<draw:path svg:viewBox="0 0 0 0" svg:d="M0 0 {long}"/>"#),
        r#"<draw:path svg:viewBox="a b c" svg:d="M1e40 NaN C 1 2 3 Z Z Z m l h v q t s a 1 1 1 1 1 1"/>"#.to_string(),
        r#"<draw:path svg:viewBox="0 0 100 100" svg:d="M0 0 A0 0 0 1 1 100 100 A1e30 1e30 1e30 0 0 -1e30 5"/>"#.to_string(),
        r#"<draw:polygon svg:viewBox="0 0 -5 1e40" draw:points="1,2,3 ,, 4"/>"#.to_string(),
        r#"<draw:regular-polygon svg:width="1in" svg:height="1in" draw:corners="4000000000" draw:concave="true" draw:sharpness="-1e30%"/>"#
            .to_string(),
        r#"<draw:regular-polygon draw:corners="0"/><draw:regular-polygon draw:corners="1"/>"#.to_string(),
    ] {
        let d = open(&el);
        for (_, _, paths) in drawn(&d) {
            for (pts, _) in paths {
                assert!(pts.len() <= wordcraft_doc::freeform::MAX_POINTS && pts.iter().all(|(x, y)| x.is_finite() && y.is_finite()));
            }
        }
        let _ = export(&d).unwrap();
    }
}
