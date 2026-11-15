use super::*;
use serde_json::json;

/// An opaque `w` × `h` mid-grey picture.
fn grey(w: u32, h: u32) -> Canvas {
    Canvas::new(RgbaImage::from_pixel(w, h, image::Rgba([128, 128, 128, 255])))
}

/// One pixel per point.
fn unit() -> Scale {
    Scale::new((100, 100), (100.0, 100.0))
}

fn alpha(cv: &Canvas, x: u32, y: u32) -> u8 {
    cv.img.get_pixel(x, y).0[3]
}

fn effects(v: serde_json::Value) -> PictureEffects {
    let mut fx = PictureEffects::default();
    change(&v, &mut fx).unwrap();
    fx
}

#[test]
fn every_shape_is_positive_inside_and_negative_outside() {
    for shape in [Shape::Rect, Shape::Rounded(0.2), Shape::Oval, Shape::SnipDiagonal(0.2), Shape::RoundDiagonal(0.2)] {
        assert!(shape.depth(50.0, 30.0, 100.0, 60.0) > 20.0, "{shape:?}: the centre is deep inside");
        assert!(shape.depth(-3.0, 30.0, 100.0, 60.0) < 0.0, "{shape:?}: left of it");
        assert!(shape.depth(50.0, 63.0, 100.0, 60.0) < 0.0, "{shape:?}: below it");
        assert!(shape.depth(f32::NAN, 0.0, 0.0, 0.0).is_nan() || shape.depth(f32::NAN, 0.0, 0.0, 0.0).is_finite(), "never panics");
    }
    // Corners: a rectangle keeps them, the rounded and clipped shapes don't.
    assert!(Shape::Rect.depth(1.0, 1.0, 100.0, 60.0) > 0.0);
    assert!(Shape::Rounded(0.3).depth(1.0, 1.0, 100.0, 60.0) < 0.0);
    assert!(Shape::SnipDiagonal(0.3).depth(1.0, 1.0, 100.0, 60.0) < 0.0);
    assert!(Shape::SnipDiagonal(0.3).depth(99.0, 1.0, 100.0, 60.0) > 0.0, "only the diagonal corners are clipped");
    assert!(Shape::RoundDiagonal(0.3).depth(99.0, 59.0, 100.0, 60.0) < 0.0);
    assert!(Shape::Oval.depth(2.0, 2.0, 100.0, 60.0) < 0.0);
}

#[test]
fn presets_and_parts_parse_and_unknown_names_are_refused() {
    let fx = effects(
        json!({"shadow": "offsetBottomRight", "reflection": "half4pt", "glow": 8, "softEdge": 5, "bevel": "circle", "rotation": "perspectiveLeft"}),
    );
    assert!(fx.shadow.is_some_and(|s| !s.inner && s.distance > 0.0));
    assert_eq!(fx.reflection.map(|r| r.distance), Some(4.0));
    assert_eq!(fx.glow.map(|g| g.size), Some(8.0));
    assert_eq!(fx.soft_edge, Some(5.0));
    assert_eq!(fx.bevel.map(|b| b.kind), Some(BevelKind::Circle));
    assert!(fx.rotation.is_some_and(|r| r.perspective > 0.0));
    // An object changes only the parts it names, of the effect there already.
    let mut more = fx;
    change(&json!({"shadow": {"blur": 10}, "glow": {"color": "FF0000"}}), &mut more).unwrap();
    assert_eq!(more.shadow.map(|s| (s.blur, s.distance)), fx.shadow.map(|s| (10.0, s.distance)));
    assert_eq!(more.glow.map(|g| (g.color, g.size)), Some((Rgb(255, 0, 0), 8.0)));
    // Left out stays; null and "none" take it off.
    change(&json!({"shadow": null, "reflection": "none", "softEdge": null}), &mut more).unwrap();
    assert!(more.shadow.is_none() && more.reflection.is_none() && more.soft_edge.is_none());
    assert!(more.glow.is_some() && more.bevel.is_some(), "the others stay");
    // A preset replaces them all.
    change(&json!({"preset": "preset1"}), &mut more).unwrap();
    assert_eq!(Some(more), preset(1));
    change(&json!({"preset": "none"}), &mut more).unwrap();
    assert!(more.is_empty());
    for bad in [
        json!({"shadow": "sparkly"}),
        json!({"bevel": "round"}),
        json!({"rotation": "sideways"}),
        json!({"preset": "preset13"}),
        json!({"glow": [1]}),
    ] {
        assert!(matches!(change(&bad, &mut more), Err(CmdError::Params(_))), "{bad}");
    }
}

#[test]
fn hostile_numbers_are_clamped() {
    let fx = effects(json!({
        "shadow": {"blur": 1e30, "distance": -5, "angle": 1e20, "transparency": 400, "size": 0},
        "reflection": {"size": 1e9, "distance": -1, "blur": f64::MAX},
        "glow": {"size": 1e9, "transparency": -50},
        "softEdge": 1e12,
        "bevel": {"width": 1e9, "height": -3},
        "rotation": {"x": 1e9, "y": -1e9, "z": -1e30, "perspective": 1e9},
    }));
    let s = fx.shadow.unwrap();
    assert!(s.blur <= MAX_LEN && s.distance >= 0.0 && (0.0..360.0).contains(&s.angle) && s.transparency <= 100.0 && s.size >= 1.0);
    let r = fx.reflection.unwrap();
    assert!(r.size <= 100.0 && r.distance >= 0.0 && r.blur <= MAX_LEN);
    assert!(fx.glow.is_some_and(|g| g.size <= MAX_LEN && g.transparency >= 0.0));
    assert!(fx.soft_edge.is_some_and(|r| r <= MAX_LEN));
    assert!(fx.bevel.is_some_and(|b| b.width <= MAX_LEN && b.height >= 0.0));
    let rot = fx.rotation.unwrap();
    assert!(rot.x.abs() <= MAX_TURN && rot.y.abs() <= MAX_TURN && (0.0..360.0).contains(&rot.z) && rot.perspective <= 120.0);
    // Drawn at a huge scale, everything stays within the bitmap budget or is refused.
    let mut cv = grey(64, 48);
    let sc = Scale::new((64, 48), (0.001, 0.001));
    match draw(&mut cv, &fx, sc) {
        Ok(()) => assert!(u64::from(cv.img.width()) * u64::from(cv.img.height()) <= MAX_CANVAS_PIXELS),
        Err(e) => assert!(e.to_string().contains("too large"), "{e}"),
    }
}

#[test]
fn a_shadow_is_cast_where_it_points_and_the_room_follows() {
    let mut cv = grey(40, 30);
    draw(&mut cv, &effects(json!({"shadow": {"preset": "offsetBottomRight", "distance": 6, "blur": 2, "transparency": 0}})), unit()).unwrap();
    let [l, t, r, b] = cv.room();
    assert!(r >= 6 && b >= 6, "room on the side it is cast: {:?}", cv.room());
    assert!(l < r && t < b, "less on the other side: {:?}", cv.room());
    // Beside the picture, toward the shadow, it is dark; on the far side, clear.
    let (x0, y0) = cv.at;
    let shade = cv.img.get_pixel(x0 + 40 + 3, y0 + 15).0;
    assert!(shade[3] > 100 && shade[0] < 60, "shadow right of the picture: {shade:?}");
    assert!(x0 == 0 || alpha(&cv, x0 - 1, y0 + 15) < 40, "little to the left");
    // The picture itself is as it was.
    assert_eq!(cv.img.get_pixel(x0 + 20, y0 + 15).0, [128, 128, 128, 255]);
}

#[test]
fn a_shadow_follows_a_transparent_pictures_shape() {
    // A disc on a clear square: the shadow is a disc too, so the square's corner casts none.
    let img = RgbaImage::from_fn(60, 60, |x, y| {
        let d = ((x as f32 - 29.5).powi(2) + (y as f32 - 29.5).powi(2)).sqrt();
        image::Rgba([200, 30, 30, if d < 20.0 { 255 } else { 0 }])
    });
    let mut cv = Canvas::new(img);
    draw(&mut cv, &effects(json!({"shadow": {"preset": "offsetBottomRight", "distance": 4, "blur": 1, "transparency": 0}})), unit()).unwrap();
    let (x0, y0) = cv.at;
    assert!(alpha(&cv, x0 + 55, y0 + 55) < 20, "nothing behind the clear corner");
    assert!(alpha(&cv, x0 + 30 + 21, y0 + 32) > 100, "just past the disc's edge, toward the light's far side");
}

#[test]
fn a_glow_surrounds_the_picture_and_fades() {
    let mut cv = grey(30, 30);
    draw(&mut cv, &effects(json!({"glow": {"size": 10, "color": "FF0000", "transparency": 0}})), unit()).unwrap();
    assert!(cv.room().iter().all(|r| *r >= 10), "{:?}", cv.room());
    let (x0, y0) = cv.at;
    let near = cv.img.get_pixel(x0 - 2, y0 + 15).0;
    let far = cv.img.get_pixel(x0 - 9, y0 + 15).0;
    assert!(near[0] > 200 && near[1] < 40 && near[3] > far[3], "red, stronger near: {near:?} {far:?}");
    assert!(cv.img.get_pixel(0, 0).0[3] < 30, "faded out at the corner");
}

#[test]
fn a_reflection_lies_below_and_fades_out() {
    let img = RgbaImage::from_fn(20, 40, |_, y| image::Rgba(if y < 20 { [255, 0, 0, 255] } else { [0, 0, 255, 255] }));
    let mut cv = Canvas::new(img);
    draw(&mut cv, &effects(json!({"reflection": {"size": 50, "distance": 4, "transparency": 0, "blur": 0}})), unit()).unwrap();
    let [l, t, r, b] = cv.room();
    assert_eq!((l, t, r), (0, 0, 0));
    assert_eq!(b, 4 + 20, "distance and half the height");
    let (x0, y0) = cv.at;
    assert_eq!(alpha(&cv, x0 + 10, y0 + 41), 0, "the gap");
    let first = cv.img.get_pixel(x0 + 10, y0 + 44).0;
    assert!(first[2] > 200 && first[0] < 30 && first[3] > 200, "mirrored: the bottom (blue) first: {first:?}");
    assert!(alpha(&cv, x0 + 10, y0 + 63) < first[3] / 4, "fading out");
}

#[test]
fn soft_edges_fade_toward_the_outline() {
    let mut cv = grey(40, 40);
    draw(&mut cv, &effects(json!({"softEdge": 8})), unit()).unwrap();
    assert_eq!(cv.room(), [0; 4], "soft edges stay inside");
    assert!(alpha(&cv, 0, 20) < 20 && alpha(&cv, 4, 20) < alpha(&cv, 7, 20) && alpha(&cv, 20, 20) == 255);
}

#[test]
fn a_bevel_lights_the_top_left_edge_and_shades_the_bottom_right() {
    for (id, _) in BEVELS {
        let mut cv = grey(60, 60);
        draw(&mut cv, &effects(json!({"bevel": {"preset": id, "width": 8, "height": 8}})), unit()).unwrap();
        assert_eq!(cv.room(), [0; 4]);
        assert_eq!(cv.img.get_pixel(30, 30).0, [128, 128, 128, 255], "{id}: the middle is flat");
        let changed = cv.img.pixels().filter(|p| p.0[0] != 128).count();
        assert!(changed > 60, "{id}: the edge is shaded");
    }
    let mut cv = grey(60, 60);
    draw(&mut cv, &effects(json!({"bevel": "circle"})), unit()).unwrap();
    assert!(cv.img.get_pixel(2, 30).0[0] > 128 && cv.img.get_pixel(57, 30).0[0] < 128, "lit left, shaded right");
}

#[test]
fn an_inner_shadow_darkens_inside_its_edge_only() {
    let mut cv = grey(50, 50);
    draw(&mut cv, &effects(json!({"shadow": {"preset": "insideTopLeft", "transparency": 0}})), unit()).unwrap();
    assert_eq!(cv.room(), [0; 4], "inside only");
    assert!(cv.img.get_pixel(1, 25).0[0] < 100, "dark along the left edge");
    assert!(cv.img.get_pixel(48, 25).0[0] >= 120, "not along the right");
}

#[test]
fn a_3d_rotation_reshapes_the_picture_round_its_window() {
    let mut cv = grey(80, 60);
    draw(&mut cv, &effects(json!({"rotation": "perspectiveLeft"})), unit()).unwrap();
    let (w, h) = cv.img.dimensions();
    assert!(w >= 80 && h >= 60, "the window still fits: {w} x {h}");
    let (x0, y0) = cv.at;
    // Turned with its right side away, its left edge is taller than its right.
    let column = |x: u32| (0..h).filter(|y| cv.img.get_pixel(x, *y).0[3] > 128).count();
    assert!(column(x0 + 3) > column(x0 + 76), "{} vs {}", column(x0 + 3), column(x0 + 76));
    assert!(alpha(&cv, x0 + 40, y0 + 30) == 255, "the middle is still the picture");
    // Every preset draws, none past the budget.
    for (id, _) in PARALLEL.iter().chain(&PERSPECTIVES) {
        let mut cv = grey(30, 20);
        draw(&mut cv, &effects(json!({"rotation": id})), unit()).unwrap();
        assert!(cv.img.pixels().any(|p| p.0[3] > 0), "{id}");
    }
}

#[test]
fn rings_go_round_the_outline_and_dashes_leave_gaps() {
    let mut cv = grey(40, 20);
    cv.ring(Rgb(255, 0, 0), 3.0, Dash::Solid, false).unwrap();
    assert_eq!(cv.img.dimensions(), (46, 26));
    assert_eq!(cv.room(), [3; 4]);
    assert_eq!(cv.img.get_pixel(0, 0).0, [255, 0, 0, 255], "a whole pixel width stays sharp");
    assert_eq!(cv.img.get_pixel(10, 10).0, [128, 128, 128, 255], "the picture inside");
    let mut dashed = grey(40, 20);
    dashed.ring(Rgb(0, 0, 255), 2.0, Dash::Dash, false).unwrap();
    let top: Vec<u8> = (0..44).map(|x| dashed.img.get_pixel(x, 0).0[3]).collect();
    assert!(top.contains(&0) && top.contains(&255), "dashes and gaps along the top: {top:?}");
    // A clear part of the picture stays clear inside its border.
    let mut clear = Canvas::new(RgbaImage::from_pixel(10, 10, image::Rgba([0, 0, 0, 0])));
    clear.ring(Rgb::BLACK, 2.0, Dash::Solid, false).unwrap();
    assert_eq!(alpha(&clear, 6, 6), 0);
}

#[test]
fn every_preset_draws_on_a_small_picture() {
    for n in 1..=PRESETS {
        let fx = preset(n).unwrap();
        let mut cv = grey(24, 16);
        draw(&mut cv, &fx, Scale::new((24, 16), (24.0, 16.0))).unwrap();
        assert!(cv.img.pixels().any(|p| p.0[3] > 0), "preset{n}");
    }
    assert!(preset(0).is_none() && preset(PRESETS + 1).is_none());
}
