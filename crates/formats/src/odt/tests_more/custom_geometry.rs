//! Custom shapes whose type WordCraft has no kind for (ODF 1.3 §10.6.2
//! `draw:enhanced-geometry`): drawn from their own path (§19.145 `draw:enhanced-path`), its
//! values worked out from the shape's equations (§10.6.3 `draw:equation`) and adjustment values
//! (`draw:modifiers`), as freeforms.

use crate::odt::tests::{NS_T, package};
use crate::odt::{export, import};
use wordcraft_doc::Document;
use wordcraft_doc::para::{InlineObject, ShapeKind};

const NS_D: &str =
    r#"xmlns:draw="urn:oasis:names:tc:opendocument:xmlns:drawing:1.0" xmlns:svg="urn:oasis:names:tc:opendocument:xmlns:svg-compatible:1.0""#;

/// A 1in square custom shape with geometry attributes `geo` and equations `eqs`.
fn shape(geo: &str, eqs: &str) -> String {
    format!(
        r#"<draw:custom-shape text:anchor-type="paragraph" svg:width="1in" svg:height="1in"><draw:enhanced-geometry {geo}>{eqs}</draw:enhanced-geometry></draw:custom-shape>"#
    )
}

fn open(shapes: &str) -> Document {
    let content = format!(
        r#"<office:document-content {NS_T} {NS_D}><office:body><office:text><text:p>{shapes}Text.</text:p></office:text></office:body></office:document-content>"#
    );
    import(&package(&content, None)).unwrap()
}

/// Each shape's kind and its paths in frame points (rounded), closed or not.
#[allow(clippy::type_complexity)]
fn drawn(d: &Document) -> Vec<(ShapeKind, Vec<(Vec<(f32, f32)>, bool)>)> {
    let r = |v: f32| v.round();
    d.body
        .iter()
        .filter_map(|b| b.as_para())
        .flat_map(|p| p.objects.iter())
        .filter_map(|o| match o {
            InlineObject::Shape { kind, w, h, freeform, .. } => {
                let paths = freeform
                    .as_ref()
                    .map(|f| f.placed(0.0, 0.0, *w, *h).into_iter().map(|(pts, c)| (pts.iter().map(|[x, y]| (r(*x), r(*y))).collect(), c)).collect())
                    .unwrap_or_default();
                Some((*kind, paths))
            }
            _ => None,
        })
        .collect()
}

#[test]
fn a_hexagon_is_drawn_from_its_equations_and_adjustment_value() {
    let d = open(&shape(
        r#"svg:viewBox="0 0 21600 21600" draw:type="hexagon" draw:modifiers="5400" draw:enhanced-path="M ?f0 0 L ?f1 0 21600 10800 ?f1 21600 ?f0 21600 0 10800 Z N""#,
        r#"<draw:equation draw:name="f0" draw:formula="$0 "/><draw:equation draw:name="f1" draw:formula="right-$0"/>"#,
    ));
    let pts = vec![(18.0, 0.0), (54.0, 0.0), (72.0, 36.0), (54.0, 72.0), (18.0, 72.0), (0.0, 36.0)];
    let got = drawn(&d);
    assert_eq!(got.len(), 1);
    assert_eq!(got[0].0, ShapeKind::Freeform);
    let (p, closed) = &got[0].1[0];
    assert!(*closed);
    assert_eq!(p.get(..6), Some(pts.as_slice()), "{p:?}");
}

#[test]
fn formulas_use_functions_operators_and_other_equations() {
    // f2 = if(f1, 10800, 0) with f1 = max(abs(-3), sqrt(16)) * (2 + 1) / 4 = 3: 10800.
    let d = open(&shape(
        r#"draw:type="mso-spt99" draw:enhanced-path="M 0 0 L ?f2 ?f3 N""#,
        r#"<draw:equation draw:name="f1" draw:formula="max(abs(-3),sqrt(16))*(2+1)/4"/><draw:equation draw:name="f2" draw:formula="if(?f1 ,width/2,0)"/><draw:equation draw:name="f3" draw:formula="-(-?f2)+0/0"/>"#,
    ));
    assert_eq!(drawn(&d)[0].1, [(vec![(0.0, 0.0), (36.0, 36.0)], false)]);
}

#[test]
fn quarter_ellipses_run_along_one_axis_then_the_other() {
    // A terminator: straight top and bottom, rounded ends.
    let d = open(&shape(
        r#"draw:type="flowchart-terminator" draw:enhanced-path="M 3470 21600 X 0 10800 3470 0 L 18130 0 X 21600 10800 18130 21600 Z N""#,
        "",
    ));
    let (_, paths) = &drawn(&d)[0];
    let pts = &paths[0].0;
    // Bulging out to the frame's left and right edges, never beyond.
    let (min_x, max_x) = pts.iter().fold((f32::MAX, f32::MIN), |(a, b), p| (a.min(p.0), b.max(p.0)));
    assert_eq!((min_x, max_x), (0.0, 72.0));
    assert!(pts.contains(&(0.0, 36.0)) && pts.contains(&(72.0, 36.0)), "{pts:?}");
    // The left end leaves the bottom point going left (along x) first: its next point is lower
    // than half way.
    let next = pts.get(1).copied().unwrap_or_default();
    assert!(next.0 < 12.0 && next.1 > 54.0, "{next:?}");
}

#[test]
fn ellipses_and_arcs_are_followed_in_steps() {
    // A circle (U: centre, radii, angles), and the upper half of the frame's ellipse, drawn
    // anticlockwise from its right to its left (A: box, from, to).
    let d =
        open(&shape(r#"draw:type="smiley" draw:enhanced-path="U 10800 10800 10800 10800 0 360 Z N B 0 0 21600 21600 21600 10800 0 10800 N""#, ""));
    let (_, paths) = &drawn(&d)[0];
    assert_eq!(paths.len(), 2, "{paths:?}");
    let circle = &paths[0].0;
    assert!(circle.len() > 32 && circle.iter().all(|(x, y)| ((x - 36.0).hypot(y - 36.0) - 36.0).abs() <= 1.0), "{circle:?}");
    let arc = &paths[1].0;
    assert_eq!((arc.first(), arc.last()), (Some(&(72.0, 36.0)), Some(&(0.0, 36.0))));
    // Anticlockwise on the page (y down): over the top.
    assert!(arc.iter().all(|(_, y)| *y <= 36.0), "{arc:?}");
}

#[test]
fn a_custom_shape_keeps_its_drawn_geometry_when_saved() {
    let d = open(&shape(r#"draw:type="octagon" draw:enhanced-path="M 0 0 L 21600 0 10800 21600 Z N""#, ""));
    let again = import(&export(&d).unwrap()).unwrap();
    assert_eq!(drawn(&again), drawn(&d));
    assert_eq!(drawn(&again)[0].0, ShapeKind::Freeform);
}

#[test]
fn known_and_plain_types_keep_their_kinds() {
    let d = open(&format!(
        "{}{}",
        shape(r#"draw:type="ellipse" draw:enhanced-path="M 0 0 L 1 1 N""#, ""),
        shape(r#"draw:type="rectangle" draw:enhanced-path="M 0 0 L 21600 0 21600 21600 0 21600 Z N""#, "")
    ));
    let kinds: Vec<ShapeKind> = drawn(&d).iter().map(|s| s.0).collect();
    assert_eq!(kinds, [ShapeKind::Ellipse, ShapeKind::Rectangle]);
}

#[test]
fn hostile_geometry_never_panics_and_draws_what_it_can() {
    let deep = format!("{}1{}", "(".repeat(5000), ")".repeat(5000));
    let unary = "-".repeat(5000);
    let mut eqs = String::new();
    // A chain of equations each referring to the next, and one referring to itself.
    for i in 0..2000 {
        eqs.push_str(&format!(r#"<draw:equation draw:name="f{i}" draw:formula="?f{} +1"/>"#, i + 1));
    }
    eqs.push_str(r#"<draw:equation draw:name="me" draw:formula="?me"/>"#);
    for (path, eq) in [
        ("M ?f0 0 L 10 10 N", eqs.as_str()),
        ("M ?me ?nothing L $99 $-1 N", eqs.as_str()),
        ("M 0 0 L ?d 1 N", &*format!(r#"<draw:equation draw:name="d" draw:formula="{deep}"/>"#)),
        ("M 0 0 L ?u 1 N", &*format!(r#"<draw:equation draw:name="u" draw:formula="{unary}1"/>"#)),
        ("M 1e308 1e308 L -1e308 1e308 C NaN inf 0 0 0 0 N", ""),
        ("U 0 0 1e300 1e300 0 1e300 Z B 0 0 0 0 0 0 0 0 W 1 1 1 1 1 1 1 1 X Y X 5 5 Y", ""),
        ("M 0 0 L 1", ""),
        ("Q", ""),
        (&*"L 1 1 ".repeat(30_000), ""),
        (&*format!("M 0 0 {}", "U 10 10 10 10 0 360 ".repeat(5000)), ""),
    ] {
        let d = open(&shape(&format!(r#"draw:type="hostile" svg:viewBox="0 0 21600 21600" draw:enhanced-path="{path}""#), eq));
        let _ = drawn(&d);
        let _ = export(&d).unwrap();
    }
    // An empty or broken view box falls back to a rectangle.
    let d = open(&shape(r#"draw:type="hostile" svg:viewBox="0 0 0 0" draw:enhanced-path="M 0 0 L 1 1 N""#, ""));
    assert_eq!(drawn(&d)[0].0, ShapeKind::Rectangle);
}

#[test]
fn a_freeform_with_open_and_closed_paths_is_saved_as_a_custom_shape() {
    // A face and a mouth: the face filled, the mouth not. A path with an open part would be
    // drawn unfilled whole, so it is saved as a custom shape whose path marks the mouth unfilled.
    let mut d = open(&shape(r#"draw:type="smiley" draw:enhanced-path="M 0 0 L 21600 0 21600 21600 Z N M 5000 15000 L 16000 15000 F N""#, ""));
    // As WordCraft's own freeform (the geometry read is written back whole otherwise).
    for b in d.body.iter_mut() {
        if let wordcraft_doc::Block::Para(p) = std::sync::Arc::make_mut(b) {
            for o in &mut p.objects {
                if let InlineObject::Shape { extra, .. } = o {
                    extra.odf_geometry = None;
                }
            }
        }
    }
    let c = crate::odt::tests::content_of(&export(&d).unwrap());
    assert!(c.contains(r#"draw:type="non-primitive""#) && c.contains(" F N"), "{c}");
    assert!(!c.contains("<draw:path"), "{c}");
    let again = import(&export(&d).unwrap()).unwrap();
    assert_eq!(drawn(&again), drawn(&d));
    assert_eq!(drawn(&again)[0].1.iter().map(|p| p.1).collect::<Vec<_>>(), [true, false]);
}

#[test]
fn equations_referring_twice_to_ones_that_fail_are_worked_out_once() {
    // Each equation refers twice to the next, and the last to itself: worked out naively, 2^200
    // steps. Each is worked out once, and all fail, so the path has nothing to draw.
    let mut eqs = String::new();
    for i in 0..200 {
        eqs.push_str(&format!(r#"<draw:equation draw:name="f{i}" draw:formula="?f{0}+?f{0}"/>"#, i + 1));
    }
    eqs.push_str(r#"<draw:equation draw:name="f200" draw:formula="?f200"/>"#);
    let start = std::time::Instant::now();
    let d = open(&shape(r#"draw:type="hostile" draw:enhanced-path="M 0 0 L ?f0 10 L 20 20 N""#, &eqs));
    assert!(start.elapsed().as_secs() < 5);
    // The line with the unknown end is left out; the rest is drawn.
    assert_eq!(drawn(&d)[0].1, [(vec![(0.0, 0.0), (0.0, 0.0)], false)]);
}

#[test]
fn points_far_outside_the_view_box_stay_finite() {
    let d = open(&shape(r#"draw:type="hostile" draw:enhanced-path="M 0 0 L 1e300 -1e300 N""#, ""));
    let Some(InlineObject::Shape { freeform: Some(f), .. }) = d.body[0].as_para().unwrap().objects.first() else { panic!("a freeform") };
    assert!(f.paths.iter().flat_map(|p| p.pts.iter()).all(|[x, y]| x.is_finite() && y.is_finite()));
}

#[test]
fn a_custom_shapes_geometry_is_written_back_as_it_was_read() {
    // LibreOffice's shape presets: saved as their own geometry (type, path, equations, handles),
    // so LibreOffice draws its own shape, not WordCraft's path of it; a preset WordCraft draws as
    // a rectangle (a cloud) keeps its geometry too. A shape made another kind loses it.
    let hexagon = shape(
        r#"svg:viewBox="0 0 21600 21600" draw:type="hexagon" draw:modifiers="5400" draw:mirror-horizontal="true" draw:enhanced-path="M ?f0 0 L ?f1 0 21600 10800 ?f1 21600 ?f0 21600 0 10800 Z N""#,
        r#"<draw:equation draw:name="f0" draw:formula="$0 "/><draw:equation draw:name="f1" draw:formula="right-$0"/><draw:handle draw:handle-position="$0 top"/>"#,
    );
    let cloud = r#"<draw:custom-shape text:anchor-type="paragraph" svg:width="1in" svg:height="1in"><draw:enhanced-geometry draw:type="cloud"/></draw:custom-shape>"#;
    let d = open(&format!("{hexagon}{cloud}"));
    assert_eq!(drawn(&d).iter().map(|s| s.0).collect::<Vec<_>>(), [ShapeKind::Freeform, ShapeKind::Rectangle]);
    let out = crate::odt::tests::content_of(&crate::odt::tests::export_model(&d).unwrap());
    assert!(out.contains(r#"draw:type="hexagon" draw:modifiers="5400""#) && out.contains(r#"draw:formula="right-$0""#), "{out}");
    assert!(out.contains(r#"<draw:handle draw:handle-position="$0 top"/>"#), "{out}");
    // Its mirroring is the shape's own, written once.
    assert_eq!(out.matches("draw:mirror-horizontal=\"true\"").count(), 1, "{out}");
    assert!(out.contains(r#"<draw:enhanced-geometry draw:type="cloud"></draw:enhanced-geometry>"#), "{out}");
    // Read back, it is the same shape again.
    let back = import(&crate::odt::tests::export_model(&d).unwrap()).unwrap();
    assert_eq!(drawn(&back), drawn(&d));
    // Made an ellipse: written as one.
    let mut changed = d.clone();
    for b in changed.body.iter_mut() {
        if let wordcraft_doc::Block::Para(p) = std::sync::Arc::make_mut(b) {
            for o in &mut p.objects {
                if let InlineObject::Shape { kind, extra, .. } = o {
                    *kind = ShapeKind::Ellipse;
                    extra.odf_geometry = None;
                }
            }
        }
    }
    let out = crate::odt::tests::content_of(&crate::odt::tests::export_model(&changed).unwrap());
    assert!(!out.contains("hexagon") && !out.contains("cloud"), "{out}");
}
