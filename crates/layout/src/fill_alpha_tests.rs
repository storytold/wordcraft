use super::*;
use wordcraft_doc::connector::ShapeExtra;
use wordcraft_doc::freeform::{FreePath, Freeform, InkTool};

#[test]
fn freeform_fill_has_own_alpha() {
    let rect = Rect::new(4.0, 4.0, 24.0, 24.0);
    for transparency in [0.0, 50.0, 100.0] {
        for closed in [false, true] {
            let freeform = Freeform {
                w: 24.0,
                h: 24.0,
                paths: vec![FreePath { pts: vec![[0.0, 0.0], [24.0, 0.0], [24.0, 24.0]], closed }],
                alpha: 0.25,
                ..Default::default()
            };
            let mut out = Vec::new();
            shape_draws(
                rect,
                ShapeKind::Freeform,
                Some(Rgb::BLACK),
                Some(Rgb::WHITE),
                2.0,
                Default::default(),
                Some(&freeform),
                &ShapeExtra { fill_transparency: transparency, ..Default::default() },
                &DisplayOptions::default(),
                &mut out,
            );
            let Draw::Path { fill, fill_alpha, stroke, stroke_width, .. } = &out[0] else { panic!("expected freeform path") };
            assert_eq!(*fill, closed.then_some(Rgb::BLACK));
            assert_eq!(*fill_alpha, if closed { 1.0 - transparency / 100.0 } else { 1.0 });
            assert_eq!((*stroke, *stroke_width), (Some(Rgb::WHITE), 2.0));
        }
    }
    let ink = Freeform::ink(InkTool::Highlighter, 24.0, 24.0, vec![[0.0, 0.0], [24.0, 24.0]]);
    let mut out = Vec::new();
    shape_draws(
        rect,
        ShapeKind::Freeform,
        Some(Rgb::BLACK),
        Some(Rgb::WHITE),
        2.0,
        Default::default(),
        Some(&ink),
        &ShapeExtra { fill_transparency: 100.0, ..Default::default() },
        &DisplayOptions::default(),
        &mut out,
    );
    assert!(matches!(out.as_slice(), [Draw::Ink { alpha, .. }] if *alpha == 0.5));
}

#[test]
fn other_path_alpha_defaults() {
    let rect = Rect::new(4.0, 4.0, 24.0, 24.0);
    let graphic = Graphic {
        items: vec![GraphicItem::Path {
            segs: vec![PathSeg::Move(0.0, 0.0), PathSeg::Line(24.0, 24.0)],
            fill: Some(Rgb::BLACK),
            stroke: None,
            stroke_width: 0.0,
        }],
        ..Default::default()
    };
    let mut out = graphic_draws(&Document::new(), &graphic, rect, 1.0);
    out.push(path_arrowhead(&[(4.0, 4.0), (28.0, 28.0)], Rgb::BLACK, 2.0).unwrap());
    let extra = ShapeExtra { arrow_start: true, arrow_end: true, fill_transparency: 100.0, ..Default::default() };
    line_draws(rect, ShapeKind::ElbowConnector, Some(Rgb::BLACK), 2.0, Default::default(), &extra, &mut out);
    assert_eq!(out.len(), 5);
    assert!(out.iter().all(|item| matches!(item, Draw::Path { fill_alpha, .. } if *fill_alpha == 1.0)));
}
