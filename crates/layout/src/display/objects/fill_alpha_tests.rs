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
                &ShapeLook {
                    kind: ShapeKind::Freeform,
                    fill: Some(&wordcraft_doc::fill::Fill::Solid { color: Rgb::BLACK, transparency }),
                    stroke: Some(Rgb::WHITE),
                    stroke_width: 2.0,
                    effects: Default::default(),
                    freeform: Some(&freeform),
                    extra: &ShapeExtra::default(),
                },
                &DisplayOptions::default(),
                &mut out,
            );
            let Draw::Path { fill, stroke, stroke_width, .. } = &out[0] else { panic!("expected freeform path") };
            match fill {
                Some(Paint::Solid(color, alpha)) if closed => {
                    assert_eq!(*color, Rgb::BLACK);
                    assert_eq!(*alpha, 1.0 - transparency / 100.0);
                }
                None if !closed => {}
                _ => panic!("unexpected freeform paint: {fill:?}"),
            }
            assert_eq!((*stroke, *stroke_width), (Some(Rgb::WHITE), 2.0));
        }
    }
    let ink = Freeform::ink(InkTool::Highlighter, 24.0, 24.0, vec![[0.0, 0.0], [24.0, 24.0]]);
    let mut out = Vec::new();
    shape_draws(
        rect,
        &ShapeLook {
            kind: ShapeKind::Freeform,
            fill: Some(&wordcraft_doc::fill::Fill::Solid { color: Rgb::BLACK, transparency: 100.0 }),
            stroke: Some(Rgb::WHITE),
            stroke_width: 2.0,
            effects: Default::default(),
            freeform: Some(&ink),
            extra: &ShapeExtra::default(),
        },
        &DisplayOptions::default(),
        &mut out,
    );
    assert!(matches!(out.as_slice(), [Draw::Ink { alpha, .. }] if *alpha == 0.5));
}

#[test]
fn native_taps_and_strokes_use_the_same_mean_scale_at_layout() {
    let graphic = Graphic {
        kind: GraphicKind::Ink,
        w: 11.0,
        h: 11.0,
        items: vec![
            GraphicItem::Path {
                segs: vec![PathSeg::Move(0.5, 0.5), PathSeg::Line(10.5, 0.5)],
                fill: None,
                stroke: Some(Rgb::BLACK),
                stroke_width: 1.0,
            },
            GraphicItem::Shape { rect: [5.0, 5.0, 1.0, 1.0], kind: ShapeKind::Ellipse, fill: Some(Rgb::BLACK), stroke: None, stroke_width: 0.0 },
        ],
        ..Default::default()
    };
    for (sx, sy) in [(4.0, 1.0), (1.0, 4.0), (0.5, 3.0), (2.0, 2.0)] {
        let draws = graphic_draws(&Document::new(), &graphic, Rect::new(100.0, 200.0, 11.0 * sx, 11.0 * sy), 1.0);
        let [Draw::NativeInk { width, .. }, Draw::Shape { rect, .. }] = draws.as_slice() else { panic!("{draws:?}") };
        let expected = (sx + sy) / 2.0;
        assert!((*width - expected).abs() < 1e-6);
        assert!((rect.w - expected).abs() < 1e-6 && (rect.h - expected).abs() < 1e-6);
        assert!((rect.x + rect.w / 2.0 - (100.0 + 5.5 * sx)).abs() < 1e-5);
        assert!((rect.y + rect.h / 2.0 - (200.0 + 5.5 * sy)).abs() < 1e-5);
    }
    let chart = Graphic { kind: GraphicKind::Chart, ..graphic };
    let draws = graphic_draws(&Document::new(), &chart, Rect::new(0.0, 0.0, 44.0, 11.0), 1.0);
    assert!(matches!(&draws[1], Draw::Shape { rect, .. } if rect.w == 4.0 && rect.h == 1.0));
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
    out.push(path_arrowhead(&[(4.0, 4.0), (28.0, 28.0)], Rgb::BLACK, 2.0, ArrowStyle::default()).unwrap());
    let extra = ShapeExtra { arrow_start: true, arrow_end: true, ..Default::default() };
    line_draws(rect, ShapeKind::ElbowConnector, Some(Rgb::BLACK), 2.0, Default::default(), &extra, &mut out);
    assert_eq!(out.len(), 5);
    assert!(out.iter().all(
        |item| matches!(item, Draw::Path { fill, .. } if fill.as_ref().is_none_or(|paint| matches!(paint, Paint::Solid(_, alpha) if *alpha == 1.0)))
    ));
}
