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
    render_items(&Document::new(), &items, Rect::new(0.0, 0.0, 40.0, 40.0), 4.0, &RenderOptions::default())
}

#[test]
fn legacy_ink_keeps_its_width_clamp_and_native_ink_refuses_invalid_widths() {
    for (width, expected) in [(0.01, 0.25), (400.0, 200.0)] {
        let area = Rect::new(0.0, 0.0, 500.0, 500.0);
        let pts = vec![(100.0, 250.0), (400.0, 250.0)];
        let legacy = Draw::Ink { pts: pts.clone(), color: Rgb::BLACK, width, alpha: 1.0 };
        let control = Draw::Ink { pts, color: Rgb::BLACK, width: expected, alpha: 1.0 };
        assert_eq!(legacy.ink_width(), Some(expected));
        let render = |d| render_items(&Document::new(), &[d], area, 1.0, &RenderOptions::default());
        assert_eq!(render(legacy).pixels, render(control).pixels);
    }
    for width in [f32::NAN, f32::INFINITY, -1.0, 0.0, 1_000_000.0] {
        let native = Draw::NativeInk { pts: vec![(1.0, 1.0), (2.0, 2.0)], color: Rgb::BLACK, width, alpha: 1.0 };
        assert_eq!(native.ink_width(), None);
    }
}

#[test]
fn ordinary_clips_keep_isolated_compositing() {
    let rect = Rect::new(4.0, 4.0, 24.0, 24.0);
    let red = Draw::Fill { rect, color: RED, alpha: 0.5, text_bg: false };
    let green = Draw::Fill { rect: Rect::new(16.0, 4.0, 24.0, 24.0), color: GREEN, alpha: 0.5, text_bg: false };
    let clip = Draw::Clip { rect, items: vec![red, green] };
    assert!(!contains_mask_ink(std::slice::from_ref(&clip)));
    let rendered = image(clip);
    assert_eq!(rendered.pixel(4, 4), [0, 0, 255, 255]);
    let pixel = rendered.pixel(80, 40);
    for (actual, expected) in pixel.into_iter().zip([64u8, 128, 64, 255]) {
        assert!(actual.abs_diff(expected) <= PIXEL_TOLERANCE);
    }
}

#[test]
fn ink_extreme_widths_are_not_clamped() {
    for (width, area, scale, points, column, min_rows, max_rows) in [
        (0.01, Rect::new(0.0, 0.0, 8.0, 8.0), 40.0, vec![(1.0, 4.0), (7.0, 4.0)], 160, 1, 4),
        (400.0, Rect::new(0.0, 0.0, 500.0, 500.0), 0.1, vec![(100.0, 250.0), (400.0, 250.0)], 25, 38, 42),
    ] {
        let draw = Draw::NativeInk { pts: points, color: Rgb::BLACK, width, alpha: 1.0 };
        let rendered = render_items(&Document::new(), &[draw], area, scale, &RenderOptions::default());
        let rows = (0..rendered.height).filter(|y| rendered.pixel(column, *y)[3] > 0).count();
        assert!((min_rows..=max_rows).contains(&rows), "width={width}, rows={rows}");
    }
}

#[test]
fn mask_ink_obeys_the_shared_path_extent_guard() {
    let draw = |extent: f32| Draw::MaskInk {
        segs: vec![PathSeg::Move(0.0, 0.0), PathSeg::Cubic(extent, 0.0, extent, extent, 0.0, extent), PathSeg::Close],
        color: Rgb::BLACK,
        alpha: 1.0,
    };
    for (extent, scale) in [(MAX_PATH_PIXELS as f32 * 2.0, 1.0), (MAX_PATH_PIXELS as f32 / 2.0, 4.0)] {
        let rendered = render_items(&Document::new(), &[draw(extent)], Rect::new(0.0, 0.0, 8.0, 8.0), scale, &RenderOptions::default());
        assert!(rendered.pixels.as_chunks::<4>().0.iter().all(|p| p[3] == 0), "oversized mask was painted");
    }
    let rendered = render_items(&Document::new(), &[draw(8.0)], Rect::new(0.0, 0.0, 8.0, 8.0), 1.0, &RenderOptions::default());
    assert!(rendered.pixel(4, 4)[3] > 0);
}

#[test]
fn binary_mask_dark_mode_inverts_the_composite_not_just_the_pen() {
    let bounds = Rect::new(0.0, 0.0, 20.0, 20.0);
    let segs = vec![PathSeg::Move(2.0, 2.0), PathSeg::Line(18.0, 2.0), PathSeg::Line(18.0, 18.0), PathSeg::Line(2.0, 18.0), PathSeg::Close];
    for dark in [false, true] {
        for color in [Rgb(0, 0, 0), Rgb(255, 255, 255), Rgb(255, 255, 0), Rgb(0, 255, 0), Rgb(0, 0, 255)] {
            for alpha in [0.0, 0.5, 1.0] {
                let items =
                    [Draw::Fill { rect: bounds, color: Rgb::WHITE, alpha: 1.0, text_bg: false }, Draw::MaskInk { segs: segs.clone(), color, alpha }];
                let opts = RenderOptions { dark, ..Default::default() };
                let pixel = render_items(&Document::new(), &items, bounds, 1.0, &opts).pixel(10, 10);
                for (actual, component) in pixel.into_iter().take(3).zip([color.0, color.1, color.2]) {
                    let expected = if dark {
                        if component == 0 { 51.0 + 204.0 * alpha } else { 51.0 }
                    } else if component == 0 {
                        255.0 * (1.0 - alpha)
                    } else {
                        255.0
                    };
                    assert!((f32::from(actual) - expected).abs() <= 2.0, "dark={dark}, {color:?}, alpha={alpha}, pixel={pixel:?}");
                }
            }
        }
    }
}

#[test]
fn nested_clips_blend_masks_against_the_actual_backdrop() {
    let area = Rect::new(0.0, 0.0, 20.0, 20.0);
    let background = Draw::Fill { rect: area, color: Rgb(50, 100, 150), alpha: 1.0, text_bg: false };
    let segs = vec![PathSeg::Move(1.0, 1.0), PathSeg::Line(19.0, 1.0), PathSeg::Line(19.0, 19.0), PathSeg::Line(1.0, 19.0), PathSeg::Close];
    for dark in [false, true] {
        let opts = RenderOptions { dark, ..Default::default() };
        let control = render_items(&Document::new(), std::slice::from_ref(&background), area, 1.0, &opts);
        let baseline = control.pixel(10, 10);
        for color in [Rgb::BLACK, Rgb::WHITE, Rgb(255, 255, 0)] {
            for alpha in [0.5, 1.0] {
                let mask = Draw::MaskInk { segs: segs.clone(), color, alpha };
                let inner = Draw::Clip { rect: Rect::new(4.0, 4.0, 12.0, 12.0), items: vec![mask] };
                let outer = Draw::Clip { rect: Rect::new(2.0, 2.0, 16.0, 16.0), items: vec![inner] };
                let rendered = render_items(&Document::new(), &[background.clone(), outer], area, 1.0, &opts);
                let pixel = rendered.pixel(10, 10);
                for ((actual, original), component) in pixel.into_iter().zip(baseline).take(3).zip([color.0, color.1, color.2]) {
                    let expected = if component == 255 {
                        f32::from(original)
                    } else if dark {
                        f32::from(original) + (255.0 - f32::from(original)) * alpha
                    } else {
                        f32::from(original) * (1.0 - alpha)
                    };
                    assert!((f32::from(actual) - expected).abs() <= 2.0, "dark={dark}, {color:?}, alpha={alpha}, pixel={pixel:?}");
                }
                assert_eq!(rendered.pixel(3, 10), control.pixel(3, 10));
                assert_eq!(rendered.pixel(18, 10), control.pixel(18, 10));
            }
        }
    }
}

#[test]
fn rotated_clips_preserve_backdrop_blending_and_clip_boundaries() {
    let area = Rect::new(0.0, 0.0, 20.0, 20.0);
    let background = Draw::Fill { rect: area, color: Rgb::WHITE, alpha: 1.0, text_bg: false };
    let mask = Draw::MaskInk {
        segs: vec![PathSeg::Move(1.0, 1.0), PathSeg::Line(19.0, 1.0), PathSeg::Line(19.0, 19.0), PathSeg::Line(1.0, 19.0), PathSeg::Close],
        color: Rgb::BLACK,
        alpha: 1.0,
    };
    let clip = Draw::Clip { rect: Rect::new(5.0, 7.0, 10.0, 6.0), items: vec![mask] };
    let rotated = Draw::Rotated { cx: 10.0, cy: 10.0, spin: wordcraft_geom::Spin::new(90.0, false, false), items: vec![clip] };
    for (dark, baseline, painted) in [(false, 255, 0), (true, 51, 255)] {
        let opts = RenderOptions { dark, ..Default::default() };
        let image = render_items(&Document::new(), &[background.clone(), rotated.clone()], area, 1.0, &opts);
        assert_eq!(image.pixel(10, 6), [painted, painted, painted, 255]);
        assert_eq!(image.pixel(5, 10), [baseline, baseline, baseline, 255]);
        assert_eq!(image.pixel(10, 16), [baseline, baseline, baseline, 255]);
    }
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
        fill: Some(wordcraft_layout::display::Paint::Solid(RED, alpha)),
        stroke: None,
        stroke_width: 0.0,
        dash: Vec::new(),
        cap: Default::default(),
        corner: None,
        effects: ShapeEffects { soft_edge: Some(radius), ..effects },
    }
}

#[test]
fn path_fill_keeps_stroke_alpha() {
    for alpha in [0.0, 0.5, 1.0] {
        let item = Draw::Path {
            segs: vec![PathSeg::Move(4.0, 4.0), PathSeg::Line(28.0, 4.0), PathSeg::Line(28.0, 28.0), PathSeg::Line(4.0, 28.0), PathSeg::Close],
            fill: Some(wordcraft_layout::display::Paint::Solid(RED, alpha)),
            stroke: Some(GREEN),
            stroke_width: 2.0,
            dash: Vec::new(),
            cap: Default::default(),
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

#[test]
fn caret_samples_soft_edge_bands() {
    for kind in [ShapeKind::Rectangle, ShapeKind::Ellipse] {
        for alpha in [0.0, 0.5, 1.0] {
            let shape = soft_shape(kind, alpha, 6.0, ShapeEffects::default());
            let rendered = image(shape.clone());
            for (x, y) in [(18, 64), (22, 64), (30, 64), (64, 64)] {
                let mut bg = [0.0, 0.0, 255.0];
                let point = wordcraft_geom::Point::new((x as f32 + 0.5) / 4.0, (y as f32 + 0.5) / 4.0);
                blend_backgrounds(std::slice::from_ref(&shape), point, &RenderOptions::default(), &mut bg, 0);
                let pixel = rendered.pixel(x, y);
                for (sample, channel) in bg.into_iter().zip(pixel) {
                    assert!((sample - channel as f32).abs() <= PIXEL_TOLERANCE as f32, "{kind:?}, alpha={alpha}, ({x},{y}): {bg:?} vs {pixel:?}");
                }
            }
        }
    }
}
