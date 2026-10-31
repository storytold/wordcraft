use super::*;

/// Points along a closed polygon, `per` to a side, with a little deterministic hand wobble.
fn around(corners: &[[f32; 2]], per: usize) -> Vec<[f32; 2]> {
    let mut out = Vec::new();
    for (i, a) in corners.iter().enumerate() {
        let b = corners[(i + 1) % corners.len()];
        for k in 0..per {
            let t = k as f32 / per as f32;
            let wob = (out.len() as f32 * 1.7).sin() * 0.6;
            out.push([a[0] + (b[0] - a[0]) * t + wob, a[1] + (b[1] - a[1]) * t - wob]);
        }
    }
    out.push(corners[0]);
    out
}

#[test]
fn rectangles_ellipses_lines_and_triangles_are_recognised() {
    // A rectangle, started mid-side as people often do.
    let rect = around(&[[50.0, 10.0], [110.0, 10.0], [110.0, 70.0 + 0.0], [10.0, 70.0], [10.0, 10.0]], 12);
    let Some(InkShape::Rectangle { x, y, w, h }) = recognise(&rect) else { panic!("{:?}", recognise(&rect)) };
    assert!((x - 10.0).abs() < 2.0 && (y - 10.0).abs() < 2.0 && (w - 100.0).abs() < 3.0 && (h - 60.0).abs() < 3.0, "{x} {y} {w} {h}");
    // Nearly square: a square.
    assert_eq!(recognise(&around(&[[0.0, 0.0], [60.0, 0.0], [60.0, 57.0], [0.0, 57.0]], 10)).map(|s| s.name()), Some("square"));
    // An ellipse and a circle.
    let ring = |rx: f32, ry: f32| {
        (0..=64).map(|i| (i as f32 / 64.0) * std::f32::consts::TAU).map(|a| [100.0 + rx * a.cos(), 80.0 + ry * a.sin()]).collect::<Vec<_>>()
    };
    let Some(InkShape::Ellipse { x, w, h, .. }) = recognise(&ring(40.0, 25.0)) else { panic!() };
    assert!((x - 60.0).abs() < 0.5 && (w - 80.0).abs() < 0.5 && (h - 50.0).abs() < 0.5);
    assert_eq!(recognise(&ring(30.0, 29.0)).map(|s| s.name()), Some("circle"));
    // A slightly shaky line keeps its ends.
    let line: Vec<[f32; 2]> = (0..=40).map(|i| i as f32).map(|t| [10.0 + t * 3.0, 20.0 + t + (t * 2.1).sin() * 0.8]).collect();
    assert_eq!(recognise(&line), Some(InkShape::Line { a: line[0], b: line[40] }));
    // A triangle and a pentagon.
    let tri = around(&[[0.0, 80.0], [45.0, 0.0], [90.0, 80.0]], 15);
    let Some(InkShape::Polygon(c)) = recognise(&tri) else { panic!("{:?}", recognise(&tri)) };
    assert_eq!(c.len(), 3);
    let penta: Vec<[f32; 2]> = (0..5)
        .map(|i| (i as f32 / 5.0) * std::f32::consts::TAU - std::f32::consts::FRAC_PI_2)
        .map(|a| [60.0 + 50.0 * a.cos(), 60.0 + 50.0 * a.sin()])
        .collect();
    assert_eq!(recognise(&around(&penta, 12)).map(|s| s.name()), Some("polygon"));
}

#[test]
fn scribbles_ticks_and_hostile_points_stay_ink() {
    // An open zigzag, a hook, a tap and a tiny tick.
    let zigzag: Vec<[f32; 2]> = (0..20).map(|i| [i as f32 * 8.0, if i % 2 == 0 { 0.0 } else { 30.0 }]).collect();
    assert_eq!(recognise(&zigzag), None);
    assert_eq!(recognise(&[[0.0, 0.0], [50.0, 0.0], [50.0, 50.0]]), None);
    assert_eq!(recognise(&[[5.0, 5.0]]), None);
    assert_eq!(recognise(&[[0.0, 0.0], [2.0, 3.0]]), None);
    // A closed scribble with many spikes: not a polygon of eight or fewer corners, not round.
    let star: Vec<[f32; 2]> = (0..=24)
        .map(|i| {
            let a = i as f32 / 24.0 * std::f32::consts::TAU;
            let r = if i % 2 == 0 { 60.0 } else { 20.0 };
            [80.0 + r * a.cos(), 80.0 + r * a.sin()]
        })
        .collect();
    assert_eq!(recognise(&star), None);
    // NaN, infinities and a flood of points never panic.
    let mut junk = vec![[f32::NAN, 1.0], [f32::INFINITY, 2.0], [3.0, f32::NEG_INFINITY]];
    junk.extend(std::iter::repeat_n([1.0, 1.0], MAX_POINTS * 3));
    assert_eq!(recognise(&junk), None);
    assert_eq!(recognise(&[]), None);
}

/// A stroke drawn as a shaft from `tail` to `tip`, then both barbs of a head (back to the
/// tip between them).
fn arrow_stroke(tail: [f32; 2], tip: [f32; 2], barb: f32, spread_deg: f32) -> Vec<[f32; 2]> {
    arrow_stroke_n(tail, tip, barb, spread_deg, 24, 5)
}

/// Points from `a` towards `b` (not including `b`), `n` of them.
fn line(a: [f32; 2], b: [f32; 2], n: usize) -> Vec<[f32; 2]> {
    (0..n).map(|k| k as f32 / n as f32).map(|t| [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t]).collect()
}

/// An arrow sampled as a pen samples it: about one point per point of travel.
fn dense_arrow(tail: [f32; 2], tip: [f32; 2], barb: f32, spread_deg: f32) -> Vec<[f32; 2]> {
    arrow_stroke_n(tail, tip, barb, spread_deg, dist(tail, tip).round() as usize, (barb.round() as usize).max(2))
}

/// `arrow_stroke` with `n` shaft points and `m` points per barb pass.
fn arrow_stroke_n(tail: [f32; 2], tip: [f32; 2], barb: f32, spread_deg: f32, n: usize, m: usize) -> Vec<[f32; 2]> {
    let back = (tail[1] - tip[1]).atan2(tail[0] - tip[0]);
    let barb_end = |sign: f32| {
        let a = back + sign * spread_deg.to_radians();
        [tip[0] + barb * a.cos(), tip[1] + barb * a.sin()]
    };
    let mut pts = line(tail, tip, n);
    pts.extend(line(tip, barb_end(1.0), m));
    pts.extend(line(barb_end(1.0), tip, m));
    pts.extend(line(tip, barb_end(-1.0), m));
    pts.push(barb_end(-1.0));
    pts
}

#[test]
fn arrows_drawn_in_one_stroke_are_recognised_in_any_direction() {
    for (tail, tip) in [
        ([20.0, 100.0], [180.0, 100.0]),
        ([180.0, 100.0], [20.0, 100.0]),
        ([100.0, 20.0], [100.0, 180.0]),
        ([100.0, 180.0], [100.0, 20.0]),
        ([30.0, 30.0], [150.0, 120.0]),
        ([150.0, 120.0], [30.0, 30.0]),
    ] {
        let stroke = arrow_stroke(tail, tip, 25.0, 30.0);
        let Some(InkShape::Arrow { a, b, head_at_end, head_at_start }) = recognise(&stroke) else {
            panic!("{tail:?}->{tip:?}: {:?}", recognise(&stroke))
        };
        assert!(head_at_end && !head_at_start);
        assert!(dist(a, tail) < 3.0 && dist(b, tip) < 3.0, "{a:?} {b:?} for {tail:?} {tip:?}");
        assert_eq!(recognise(&stroke).map(|s| s.name()), Some("arrow"));
    }
    // Head first, then the shaft: the same arrow.
    let mut reversed = arrow_stroke([20.0, 100.0], [180.0, 100.0], 25.0, 30.0);
    reversed.reverse();
    let Some(InkShape::Arrow { a, b, .. }) = recognise(&reversed) else { panic!("{:?}", recognise(&reversed)) };
    assert!(dist(a, [20.0, 100.0]) < 3.0 && dist(b, [180.0, 100.0]) < 3.0, "{a:?} {b:?}");
}

#[test]
fn lines_vs_barbs_hooks_and_scribbles_are_not_arrows() {
    let stroke = arrow_stroke([20.0, 100.0], [180.0, 100.0], 25.0, 30.0);
    // A plain line stays a line.
    let line: Vec<[f32; 2]> = (0..=30).map(|i| [20.0 + i as f32 * 5.0, 100.0]).collect();
    assert_eq!(recognise(&line).map(|s| s.name()), Some("line"));
    // A "V" alone is not an arrow (nor anything).
    let v = [[100.0, 100.0], [75.0, 87.5], [100.0, 100.0], [75.0, 112.5]];
    assert_eq!(recognise(&v), None);
    assert_eq!(recognise(&[[75.0, 87.5], [100.0, 100.0], [75.0, 112.5]]), None);
    // A line with one barb (a hook) is not an arrow.
    let hook = &stroke[..34];
    assert!(!matches!(recognise(hook), Some(InkShape::Arrow { .. })), "{:?}", recognise(hook));
    // Barbs outside 15..75 degrees, or too long or too short, are not heads.
    for (barb, spread) in [(25.0, 5.0), (25.0, 88.0), (100.0, 30.0), (1.0, 30.0)] {
        let s = arrow_stroke([20.0, 100.0], [180.0, 100.0], barb, spread);
        assert!(!matches!(recognise(&s), Some(InkShape::Arrow { .. })), "barb {barb} spread {spread}");
    }
    // Both barbs on the same side are not a head.
    let lopsided = [[20.0, 100.0], [180.0, 100.0], [155.0, 87.0], [180.0, 100.0], [158.0, 80.0]];
    assert!(!matches!(recognise(&lopsided), Some(InkShape::Arrow { .. })));
    // A zigzag scribble stays ink.
    let zig: Vec<[f32; 2]> = (0..9).map(|i| [i as f32 * 20.0, if i % 2 == 0 { 0.0 } else { 40.0 }]).collect();
    assert_eq!(recognise(&zig), None);
}

#[test]
fn hostile_arrow_points_never_panic() {
    let mut junk = arrow_stroke([20.0, 100.0], [180.0, 100.0], 25.0, 30.0);
    junk.insert(3, [f32::NAN, 5.0]);
    junk.push([f32::INFINITY, 1.0]);
    assert!(matches!(recognise(&junk), Some(InkShape::Arrow { .. })));
    let huge: Vec<[f32; 2]> = (0..MAX_POINTS * 2).map(|i| [(i % 977) as f32, ((i * 31) % 541) as f32]).collect();
    let _ = recognise(&huge);
    let _ = recognise(&[[1e5, 1e5], [-1e5, 1e5], [1e5, -1e5], [-1e5, -1e5], [0.0, 0.0]]);
    let _ = recognise(&[[0.0, 0.0]; 7]);
}

/// An arrow as a hand draws it: the shaft sags by `bow` and wobbles, the tip may be rounded,
/// and each barb has its own length and angle (`(length, degrees)`, left then right).
fn hand_arrow(tail: [f32; 2], tip: [f32; 2], bow: f32, round_tip: bool, barbs: [(f32, f32); 2]) -> Vec<[f32; 2]> {
    hand_arrow_n(tail, tip, bow, round_tip, barbs, 30, 6)
}

/// `hand_arrow` with `n` shaft points and `m` points per barb pass.
fn hand_arrow_n(tail: [f32; 2], tip: [f32; 2], bow: f32, round_tip: bool, barbs: [(f32, f32); 2], n: usize, m: usize) -> Vec<[f32; 2]> {
    let len = dist(tail, tip);
    let dir = [(tip[0] - tail[0]) / len, (tip[1] - tail[1]) / len];
    let normal = [-dir[1], dir[0]];
    let mut pts: Vec<[f32; 2]> = (0..n)
        .map(|i| {
            let t = i as f32 / n as f32;
            let off = bow * 4.0 * t * (1.0 - t) + (i as f32 * 1.7).sin() * 0.6;
            [tail[0] + (tip[0] - tail[0]) * t + normal[0] * off, tail[1] + (tip[1] - tail[1]) * t + normal[1] * off]
        })
        .collect();
    let back = (tail[1] - tip[1]).atan2(tail[0] - tip[0]);
    let end = |(l, deg): (f32, f32), sign: f32| {
        let a = back + sign * deg.to_radians();
        [tip[0] + l * a.cos(), tip[1] + l * a.sin()]
    };
    let seg = |a: [f32; 2], b: [f32; 2], n: usize| -> Vec<[f32; 2]> {
        (0..n).map(|k| k as f32 / n as f32).map(|t| [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t]).collect()
    };
    let (e1, e2) = (end(barbs[0], 1.0), end(barbs[1], -1.0));
    if round_tip {
        pts.extend([[tip[0] + dir[0] * 1.0 + normal[0] * 1.5, tip[1] + dir[1] * 1.0 + normal[1] * 1.5]]);
    } else {
        pts.push(tip);
    }
    pts.extend(seg(tip, e1, m));
    pts.extend(seg(e1, tip, m));
    if round_tip {
        pts.push([tip[0] + dir[0] * 1.0 - normal[0] * 1.5, tip[1] + dir[1] * 1.0 - normal[1] * 1.5]);
    }
    pts.extend(seg(tip, e2, m));
    pts.push(e2);
    pts
}

#[test]
fn hand_drawn_arrows_with_bowed_wobbly_shafts_rounded_tips_and_uneven_barbs_are_arrows() {
    let (tail, tip) = ([20.0, 100.0], [180.0, 100.0]);
    let sym = [(25.0, 30.0), (25.0, 30.0)];
    let is_arrow = |pts: &[[f32; 2]]| matches!(recognise(pts), Some(InkShape::Arrow { .. }));
    // Wobble alone (about 0.6 pt), a 4 pt sag over 160 pt, and both bowed the other way.
    assert!(is_arrow(&hand_arrow(tail, tip, 0.0, false, sym)), "wobble");
    assert!(is_arrow(&hand_arrow(tail, tip, 4.0, false, sym)), "bowed");
    assert!(is_arrow(&hand_arrow(tail, tip, -4.0, false, sym)), "bowed the other way");
    // A rounded tip, and slightly asymmetric barbs (20 at 25 degrees, 30 at 40).
    assert!(is_arrow(&hand_arrow(tail, tip, 0.0, true, sym)), "rounded tip");
    assert!(is_arrow(&hand_arrow(tail, tip, 2.0, false, [(20.0, 25.0), (30.0, 40.0)])), "asymmetric");
    // All at once, going up and to the left, and head first.
    let all = hand_arrow([180.0, 160.0], [40.0, 30.0], 4.0, true, [(20.0, 25.0), (30.0, 40.0)]);
    let Some(InkShape::Arrow { a, b, .. }) = recognise(&all) else { panic!("{:?}", recognise(&all)) };
    assert!(dist(a, [180.0, 160.0]) < 3.0 && dist(b, [40.0, 30.0]) < 3.0, "{a:?} {b:?}");
    let mut rev = all;
    rev.reverse();
    assert!(is_arrow(&rev), "head first");
}

#[test]
fn the_barb_limits_are_the_ones_that_are_detected() {
    let (tail, tip) = ([20.0, 100.0], [180.0, 100.0]); // a 160 pt shaft
    let is_arrow = |l: f32, deg: f32| matches!(recognise(&arrow_stroke(tail, tip, l, deg)), Some(InkShape::Arrow { .. }));
    // Length 5 to 50 % of the shaft (8 to 80 pt), angle 15 to 75 degrees.
    assert!(is_arrow(8.5, 30.0) && is_arrow(79.0, 30.0));
    assert!(!is_arrow(6.0, 30.0) && !is_arrow(88.0, 30.0));
    assert!(is_arrow(25.0, 17.0) && is_arrow(25.0, 73.0));
    assert!(!is_arrow(25.0, 11.0) && !is_arrow(25.0, 80.0));
}

#[test]
fn letters_and_hooks_are_not_arrows() {
    // A "Y" in both orders: arm to the fork, down the stem; stem up, then the other arm.
    let y_down = [[40.0, 0.0], [60.0, 30.0], [80.0, 0.0], [60.0, 30.0], [60.0, 90.0]];
    assert_eq!(recognise(&y_down), None);
    let y_up = [[60.0, 90.0], [60.0, 30.0], [40.0, 0.0], [60.0, 30.0], [80.0, 0.0]];
    assert_eq!(recognise(&y_up), None);
    // A print "k": stem down, up to the middle, out to the upper right, back, out to the lower right.
    let k = [[0.0, 0.0], [0.0, 100.0], [0.0, 60.0], [40.0, 30.0], [0.0, 60.0], [40.0, 100.0]];
    assert_eq!(recognise(&k), None);
    // A "k" drawn top, middle, upper right, middle, lower right.
    let k2 = [[0.0, 0.0], [0.0, 60.0], [40.0, 30.0], [0.0, 60.0], [40.0, 100.0]];
    assert_eq!(recognise(&k2), None);
    // A checkmark, an "L", a "V" and a line with one barb.
    assert_eq!(recognise(&[[0.0, 40.0], [20.0, 70.0], [90.0, 0.0]]), None);
    assert_eq!(recognise(&[[0.0, 0.0], [0.0, 80.0], [50.0, 80.0]]), None);
    let hook = &arrow_stroke([20.0, 100.0], [180.0, 100.0], 25.0, 30.0)[..34];
    assert_eq!(recognise(hook), None);
    let zig: Vec<[f32; 2]> = (0..9).map(|i| [i as f32 * 20.0, if i % 2 == 0 { 0.0 } else { 40.0 }]).collect();
    assert_eq!(recognise(&zig), None);
}

#[test]
fn a_return_that_overshoots_the_tip_still_makes_an_arrow() {
    let (tail, tip) = ([20.0, 100.0], [180.0, 100.0]);
    // Shaft to the tip, first barb, back past the tip by 2 pt, then the second barb.
    let barb = |sign: f32| {
        let a = std::f32::consts::PI + sign * 30.0f32.to_radians();
        [tip[0] + 25.0 * a.cos(), tip[1] + 25.0 * a.sin()]
    };
    let over = [tip[0] + 2.0, tip[1]];
    let line = |a: [f32; 2], b: [f32; 2], n: usize| -> Vec<[f32; 2]> {
        (0..n).map(|k| k as f32 / n as f32).map(|t| [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t]).collect()
    };
    let mut stroke = line(tail, tip, 24);
    stroke.extend(line(tip, barb(1.0), 5));
    stroke.extend(line(barb(1.0), over, 5));
    stroke.extend(line(over, barb(-1.0), 5));
    stroke.push(barb(-1.0));
    let Some(InkShape::Arrow { a, b, .. }) = recognise(&stroke) else { panic!("{:?}", recognise(&stroke)) };
    assert!(dist(a, tail) < 3.0 && dist(b, tip) < 3.0, "{a:?} {b:?}");
    // Head first: the same stroke backwards.
    stroke.reverse();
    let Some(InkShape::Arrow { a, b, .. }) = recognise(&stroke) else { panic!("{:?}", recognise(&stroke)) };
    assert!(dist(a, tail) < 3.0 && dist(b, tip) < 3.0, "{a:?} {b:?}");
}

#[test]
fn densely_sampled_arrows_keep_their_tip_and_barb_limits() {
    // A pen samples about every point; the tip must still be where the head was drawn.
    for (tail, tip) in [([20.0, 100.0], [180.0, 100.0]), ([100.0, 180.0], [100.0, 20.0]), ([150.0, 120.0], [30.0, 30.0])] {
        let stroke = dense_arrow(tail, tip, 25.0, 30.0);
        let Some(InkShape::Arrow { a, b, .. }) = recognise(&stroke) else { panic!("{tail:?}->{tip:?}: {:?}", recognise(&stroke)) };
        assert!(dist(a, tail) < 1.0 && dist(b, tip) < 1.0, "{a:?} {b:?} for {tail:?} {tip:?}");
        let mut rev = stroke;
        rev.reverse();
        let Some(InkShape::Arrow { a, b, .. }) = recognise(&rev) else { panic!("head first {tail:?}->{tip:?}: {:?}", recognise(&rev)) };
        assert!(dist(a, tail) < 1.0 && dist(b, tip) < 1.0, "head first {a:?} {b:?}");
    }
    // The documented limits hold on dense strokes too: 5 to 50 % of a 160 pt shaft, 15 to 75 degrees.
    let (tail, tip) = ([20.0, 100.0], [180.0, 100.0]);
    let is_arrow = |l: f32, deg: f32| matches!(recognise(&dense_arrow(tail, tip, l, deg)), Some(InkShape::Arrow { .. }));
    assert!(is_arrow(8.5, 30.0) && is_arrow(8.5, 45.0) && is_arrow(8.5, 17.0) && is_arrow(79.0, 30.0), "lengths");
    assert!(is_arrow(12.0, 30.0) && is_arrow(12.0, 45.0), "small barbs");
    assert!(is_arrow(25.0, 17.0) && is_arrow(25.0, 70.0) && is_arrow(25.0, 73.0), "angles");
    assert!(!is_arrow(6.0, 30.0) && !is_arrow(88.0, 30.0));
    assert!(!is_arrow(25.0, 11.0) && !is_arrow(25.0, 80.0));
    // A dense line with one barb (a hook) is still not an arrow.
    let hook = dense_arrow(tail, tip, 25.0, 30.0);
    assert!(!matches!(recognise(&hook[..186]), Some(InkShape::Arrow { .. })), "{:?}", recognise(&hook[..186]));
}

#[test]
fn densely_sampled_hand_drawn_arrows_and_overshoots_are_arrows() {
    let (tail, tip) = ([20.0, 100.0], [180.0, 100.0]);
    let sym = [(25.0, 30.0), (25.0, 30.0)];
    let dense = |bow: f32, round: bool, barbs: [(f32, f32); 2]| hand_arrow_n(tail, tip, bow, round, barbs, 160, 25);
    let tip_of = |pts: &[[f32; 2]]| match recognise(pts) {
        Some(InkShape::Arrow { b, .. }) => Some(b),
        _ => None,
    };
    for (what, pts) in [
        ("wobble", dense(0.0, false, sym)),
        ("bowed", dense(4.0, false, sym)),
        ("bowed the other way", dense(-4.0, false, sym)),
        ("rounded tip", dense(0.0, true, sym)),
        ("asymmetric", dense(2.0, false, [(20.0, 25.0), (30.0, 40.0)])),
        ("small", dense(0.0, false, [(10.0, 45.0), (10.0, 45.0)])),
        ("wide", dense(0.0, false, [(25.0, 70.0), (25.0, 70.0)])),
    ] {
        let b = tip_of(&pts);
        assert!(b.is_some_and(|b| dist(b, tip) < 2.0), "{what}: {b:?}");
        let mut rev = pts;
        rev.reverse();
        assert!(tip_of(&rev).is_some(), "{what} head first");
    }
    // Back past the tip by 2 pt between the barbs, densely sampled, both ways.
    let barb = |sign: f32| {
        let a = std::f32::consts::PI + sign * 30.0f32.to_radians();
        [tip[0] + 25.0 * a.cos(), tip[1] + 25.0 * a.sin()]
    };
    let over = [tip[0] + 2.0, tip[1]];
    let mut stroke = line(tail, tip, 160);
    stroke.extend(line(tip, barb(1.0), 25));
    stroke.extend(line(barb(1.0), over, 25));
    stroke.extend(line(over, barb(-1.0), 25));
    stroke.push(barb(-1.0));
    let b = tip_of(&stroke);
    assert!(b.is_some_and(|b| dist(b, tip) < 1.0), "overshoot: {b:?}");
    stroke.reverse();
    let b = tip_of(&stroke);
    assert!(b.is_some_and(|b| dist(b, tip) < 1.0), "overshoot head first: {b:?}");
}

/// A shaft from `tail` to `tip`, then a closed triangle head drawn round the tip in the same
/// stroke (tip, one barb end, the other, back to the tip); `n` shaft points, `m` per side.
fn closed_arrow_n(tail: [f32; 2], tip: [f32; 2], barb: f32, spread_deg: f32, n: usize, m: usize) -> Vec<[f32; 2]> {
    let [b1, b2] = barb_ends(tail, tip, barb, spread_deg);
    let mut pts = line(tail, tip, n);
    pts.extend(line(tip, b1, m));
    pts.extend(line(b1, b2, m));
    pts.extend(line(b2, tip, m));
    pts.push(tip);
    pts
}

/// The ends of a head's two barbs at `tip`, `barb` long, `spread_deg` either side of the shaft.
fn barb_ends(tail: [f32; 2], tip: [f32; 2], barb: f32, spread_deg: f32) -> [[f32; 2]; 2] {
    let back = (tail[1] - tip[1]).atan2(tail[0] - tip[0]);
    let end = |sign: f32| {
        let a = back + sign * spread_deg.to_radians();
        [tip[0] + barb * a.cos(), tip[1] + barb * a.sin()]
    };
    [end(1.0), end(-1.0)]
}

#[test]
fn arrows_with_a_closed_triangle_head_in_one_stroke_are_recognised_both_ways() {
    for (tail, tip) in
        [([20.0, 100.0], [180.0, 100.0]), ([180.0, 100.0], [20.0, 100.0]), ([100.0, 180.0], [100.0, 20.0]), ([150.0, 120.0], [30.0, 30.0])]
    {
        for stroke in [closed_arrow_n(tail, tip, 25.0, 30.0, 24, 5), closed_arrow_n(tail, tip, 20.0, 25.0, dist(tail, tip).round() as usize, 15)] {
            let Some(InkShape::Arrow { a, b, head_at_end, head_at_start }) = recognise(&stroke) else {
                panic!("{tail:?}->{tip:?}: {:?}", recognise(&stroke))
            };
            assert!(head_at_end && !head_at_start);
            assert!(dist(a, tail) < 2.0 && dist(b, tip) < 2.0, "{a:?} {b:?} for {tail:?} {tip:?}");
            // The loop round the head first, then the shaft: the same arrow.
            let mut rev = stroke;
            rev.reverse();
            let Some(InkShape::Arrow { a, b, .. }) = recognise(&rev) else { panic!("head first {tail:?}->{tip:?}: {:?}", recognise(&rev)) };
            assert!(dist(a, tail) < 2.0 && dist(b, tip) < 2.0, "head first {a:?} {b:?}");
        }
    }
    // The head size limits hold: 5 to 50 % of a 160 pt shaft, 15 to 75 degrees.
    let (tail, tip) = ([20.0, 100.0], [180.0, 100.0]);
    let is_arrow = |l: f32, deg: f32| matches!(recognise(&closed_arrow_n(tail, tip, l, deg, 24, 5)), Some(InkShape::Arrow { .. }));
    assert!(is_arrow(12.0, 30.0) && is_arrow(70.0, 30.0) && is_arrow(25.0, 20.0) && is_arrow(25.0, 70.0));
    assert!(!is_arrow(6.0, 30.0) && !is_arrow(90.0, 30.0) && !is_arrow(25.0, 8.0) && !is_arrow(25.0, 85.0));
}

#[test]
fn triangles_without_a_shaft_stay_triangles() {
    // Closed, and closed with the pen running on past the start along the first side.
    let tri = around(&[[0.0, 80.0], [45.0, 0.0], [90.0, 80.0]], 15);
    assert_eq!(recognise(&tri).map(|s| s.name()), Some("triangle"));
    let mut over = tri.clone();
    over.extend(line([0.0, 80.0], [45.0, 0.0], 15).into_iter().skip(1).take(8));
    assert!(!matches!(recognise(&over), Some(InkShape::Arrow { .. })), "{:?}", recognise(&over));
    // A long thin triangle is not a shaft with a head.
    let thin = around(&[[0.0, 0.0], [160.0, 10.0], [0.0, 20.0]], 20);
    assert!(!matches!(recognise(&thin), Some(InkShape::Arrow { .. })), "{:?}", recognise(&thin));
    // A shaft with a triangle as big as the shaft is not an arrow.
    let (tail, tip) = ([20.0, 100.0], [180.0, 100.0]);
    assert!(!matches!(recognise(&closed_arrow_n(tail, tip, 150.0, 30.0, 24, 5)), Some(InkShape::Arrow { .. })));
    // Hostile points around a closed head never panic.
    let mut junk = closed_arrow_n(tail, tip, 25.0, 30.0, 24, 5);
    junk.insert(30, [f32::NAN, f32::INFINITY]);
    assert!(matches!(recognise(&junk), Some(InkShape::Arrow { .. })));
}

/// A head drawn on its own: an open V (one barb end, the apex, the other end) or a closed
/// triangle (apex, barb end, barb end, apex), `m` points per side.
fn v_head(apex: [f32; 2], [b1, b2]: [[f32; 2]; 2], m: usize) -> Vec<[f32; 2]> {
    let mut pts = line(b1, apex, m);
    pts.extend(line(apex, b2, m));
    pts.push(b2);
    pts
}

fn triangle_head(apex: [f32; 2], [b1, b2]: [[f32; 2]; 2], m: usize) -> Vec<[f32; 2]> {
    let mut pts = line(apex, b1, m);
    pts.extend(line(b1, b2, m));
    pts.extend(line(b2, apex, m));
    pts.push(apex);
    pts
}

#[test]
fn a_head_stroke_at_a_line_end_pairs_into_an_arrow() {
    for (tail, tip) in [([20.0, 100.0], [180.0, 100.0]), ([180.0, 160.0], [40.0, 30.0]), ([100.0, 20.0], [100.0, 180.0])] {
        let shaft = line(tail, tip, 40).into_iter().chain([tip]).collect::<Vec<_>>();
        let ends = barb_ends(tail, tip, 25.0, 30.0);
        for (what, head) in [("V", v_head(tip, ends, 8)), ("triangle", triangle_head(tip, ends, 8)), ("bare V", vec![ends[0], tip, ends[1]])] {
            let Some(InkShape::Arrow { a, b, head_at_end, head_at_start }) = pair_arrow(&shaft, &head) else {
                panic!("{what} {tail:?}->{tip:?}: {:?}", pair_arrow(&shaft, &head))
            };
            assert!(head_at_end && !head_at_start);
            assert!(dist(a, tail) < 1.0 && dist(b, tip) < 1.0, "{what}: {a:?} {b:?}");
            // Drawn the other way round, the head is still found at its end.
            let mut rev = head.clone();
            rev.reverse();
            assert!(matches!(pair_arrow(&shaft, &rev), Some(InkShape::Arrow { b, .. }) if dist(b, tip) < 1.0), "{what} reversed");
            let mut back = shaft.clone();
            back.reverse();
            assert!(matches!(pair_arrow(&back, &head), Some(InkShape::Arrow { b, .. }) if dist(b, tip) < 1.0), "{what}, shaft drawn tip first");
        }
        // A triangle given as its corners (an Ink to Shape triangle), apex anywhere in the ring.
        let [b1, b2] = barb_ends(tail, tip, 25.0, 30.0);
        assert!(matches!(pair_arrow(&shaft, &[b1, b2, tip, b1]), Some(InkShape::Arrow { b, .. }) if dist(b, tip) < 1.0), "corners");
        // An apex a few points short of the end still pairs.
        let d = [(tip[0] - tail[0]) / dist(tail, tip), (tip[1] - tail[1]) / dist(tail, tip)];
        let near = [tip[0] - 6.0 * d[0] + 3.0 * d[1], tip[1] - 6.0 * d[1] - 3.0 * d[0]];
        assert!(pair_arrow(&shaft, &v_head(near, barb_ends(tail, near, 25.0, 30.0), 8)).is_some(), "near the end");
    }
}

#[test]
fn heads_away_from_the_end_the_wrong_way_or_the_wrong_size_dont_pair() {
    let (tail, tip) = ([20.0, 100.0], [180.0, 100.0]);
    let shaft = line(tail, tip, 40).into_iter().chain([tip]).collect::<Vec<_>>();
    let head_at = |apex: [f32; 2], barb: f32, deg: f32| v_head(apex, barb_ends(tail, apex, barb, deg), 8);
    // Far from either end: mid-shaft, and 40 pt past the tip.
    assert_eq!(pair_arrow(&shaft, &head_at([100.0, 100.0], 25.0, 30.0)), None);
    assert_eq!(pair_arrow(&shaft, &head_at([220.0, 100.0], 25.0, 30.0)), None);
    assert_eq!(pair_arrow(&shaft, &head_at([180.0, 140.0], 25.0, 30.0)), None);
    // Pointing back at the shaft (barbs forward), too big, too small, too narrow, too wide.
    let forward = v_head(tip, barb_ends([340.0, 100.0], tip, 25.0, 30.0), 8);
    assert_eq!(pair_arrow(&shaft, &forward), None);
    for (barb, deg) in [(100.0, 30.0), (5.0, 30.0), (25.0, 5.0), (25.0, 85.0)] {
        assert_eq!(pair_arrow(&shaft, &head_at(tip, barb, deg)), None, "barb {barb} at {deg} degrees");
    }
    // Lopsided: both barbs on one side.
    assert_eq!(pair_arrow(&shaft, &[[155.0, 87.0], tip, [158.0, 80.0]]), None);
    // The "line" must be a line, the head a head.
    let ends = barb_ends(tail, tip, 25.0, 30.0);
    let zig: Vec<[f32; 2]> = (0..9).map(|i| [20.0 + i as f32 * 20.0, if i % 2 == 0 { 100.0 } else { 120.0 }]).collect();
    assert_eq!(pair_arrow(&zig, &v_head(tip, ends, 8)), None);
    assert_eq!(pair_arrow(&shaft, &[[150.0, 80.0], [170.0, 120.0], [175.0, 80.0], [180.0, 120.0]]), None, "a scribble");
    assert_eq!(pair_arrow(&shaft, &around(&[[150.0, 60.0], [190.0, 60.0], [190.0, 140.0], [150.0, 140.0]], 8)), None, "a box");
    // Hostile input never panics.
    let mut junk = v_head(tip, ends, 8);
    junk.insert(2, [f32::NAN, 1.0]);
    junk.push([f32::INFINITY, 0.0]);
    assert!(pair_arrow(&shaft, &junk).is_some());
    assert_eq!(pair_arrow(&[], &[]), None);
    assert_eq!(pair_arrow(&shaft, &[[180.0, 100.0]]), None);
    let huge: Vec<[f32; 2]> = (0..MAX_POINTS * 2).map(|i| [(i % 977) as f32, ((i * 31) % 541) as f32]).collect();
    let _ = pair_arrow(&huge, &huge);
    let _ = pair_arrow(&shaft, &huge);
}
