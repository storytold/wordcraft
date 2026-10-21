use super::*;
use wordcraft_doc::effects::{Glow, Shadow, ShapeEffects};
use wordcraft_doc::graphic::PathSeg;
use wordcraft_geom::Rect;

const PIXEL_TOLERANCE: u8 = 3;
const BLUE: Rgb = Rgb(0, 0, 255);
const RED: Rgb = Rgb(255, 0, 0);
const GREEN: Rgb = Rgb(0, 255, 0);

fn image(item: Draw) -> Rendered {
    let items = [Draw::Fill { rect: Rect::new(0.0, 0.0, 40.0, 40.0), color: BLUE, alpha: 1.0, text_bg: false }, item];
    render_items(&Document::new(), &items, 0.0, 0.0, 40.0, 40.0, 4.0, &RenderOptions::default())
}

fn close(pixel: [u8; 4], alpha: f32) {
    let red = (255.0 * alpha).round() as u8;
    let blue = (255.0 * (1.0 - alpha)).round() as u8;
    let want = [red, 0, blue, 255];
    assert!(pixel.into_iter().zip(want).all(|(a, b)| a.abs_diff(b) <= PIXEL_TOLERANCE), "{pixel:?} != {want:?}");
}

fn soft_shape(kind: ShapeKind, alpha: f32, radius: f32, effects: ShapeEffects) -> Draw {
    Draw::Shape {
        rect: Rect::new(4.0, 4.0, 24.0, 24.0),
        kind,
        fill: Some(RED),
        fill_alpha: alpha,
        stroke: None,
        stroke_width: 0.0,
        dash: Vec::new(),
        corner: None,
        pattern: None,
        effects: ShapeEffects { soft_edge: Some(radius), ..effects },
    }
}

#[test]
fn path_fill_keeps_stroke_alpha() {
    for alpha in [0.0, 0.5, 1.0] {
        let item = Draw::Path {
            segs: vec![PathSeg::Move(4.0, 4.0), PathSeg::Line(28.0, 4.0), PathSeg::Line(28.0, 28.0), PathSeg::Line(4.0, 28.0), PathSeg::Close],
            fill: Some(RED),
            fill_alpha: alpha,
            stroke: Some(GREEN),
            stroke_width: 2.0,
            dash: Vec::new(),
        };
        let rendered = image(item);
        close(rendered.pixel(64, 64), alpha);
        assert_eq!(rendered.pixel(16, 64), [0, 255, 0, 255], "stroke at fill alpha {alpha}");
    }
    let ink = image(Draw::Ink { pts: vec![(4.0, 16.0), (28.0, 16.0)], color: RED, width: 4.0, alpha: 0.5 });
    close(ink.pixel(64, 64), 0.5);
}

#[test]
fn soft_edge_has_target_alpha() {
    for kind in [ShapeKind::Rectangle, ShapeKind::Ellipse] {
        for (radius, count) in [(3.0, 3.0), (6.0, 4.0)] {
            for alpha in [0.0, 0.5, 1.0] {
                let rendered = image(soft_shape(kind, alpha, radius, ShapeEffects::default()));
                close(rendered.pixel(64, 64), alpha);
                close(rendered.pixel(20, 64), alpha / count);
                assert_eq!(rendered.pixel(0, 64), [0, 0, 255, 255]);
            }
        }
        let opaque = image(soft_shape(kind, 1.0, 6.0, ShapeEffects::default()));
        for alpha in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY, 2.0] {
            assert_eq!(image(soft_shape(kind, alpha, 6.0, ShapeEffects::default())).pixels, opaque.pixels);
        }
        assert_eq!(
            image(soft_shape(kind, -1.0, 6.0, ShapeEffects::default())).pixels,
            image(soft_shape(kind, 0.0, 6.0, ShapeEffects::default())).pixels,
        );
    }
}

#[test]
fn soft_edge_keeps_effects() {
    let effects = ShapeEffects { shadow: Some(Shadow::default()), glow: Some(Glow::default()), soft_edge: None };
    let opaque = image(soft_shape(ShapeKind::Rectangle, 1.0, 6.0, effects));
    let clear = image(soft_shape(ShapeKind::Rectangle, 0.0, 6.0, effects));
    let mut effect_pixels = 0;
    for y in 0..160 {
        for x in 0..160 {
            if (16..112).contains(&x) && (16..112).contains(&y) {
                continue;
            }
            assert_eq!(opaque.pixel(x, y), clear.pixel(x, y));
            effect_pixels += usize::from(opaque.pixel(x, y) != [0, 0, 255, 255]);
        }
    }
    assert!(effect_pixels > 0);
    let stroke = |alpha| {
        let mut item = soft_shape(ShapeKind::Rectangle, alpha, 6.0, ShapeEffects::default());
        if let Draw::Shape { fill, stroke, stroke_width, .. } = &mut item {
            *fill = None;
            *stroke = Some(GREEN);
            *stroke_width = 2.0;
        }
        image(item)
    };
    assert_eq!(stroke(0.0).pixels, stroke(1.0).pixels);
}
