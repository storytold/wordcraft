//! Wrap outlines: the shape text follows around a floating object with Tight or Through
//! wrapping (ECMA-376 Part 1, §20.4.2.16 `wrapPolygon`, §20.4.2.17 `wrapThrough`, §20.4.2.18
//! `wrapTight`).
//!
//! A wrap polygon is given in a coordinate space where 21600 is the object's width or height.
//! Without one, an outline is derived: a shape's geometry, flattened; a picture's opaque pixels
//! (see [`alpha_outline`]); anything else its rectangle.

use serde::{Deserialize, Serialize};

use crate::para::ShapeKind;

/// The wrap polygon coordinate space: 21600 units span the object's extent on each axis.
pub const WRAP_SPACE: i32 = 21_600;
/// Most points a wrap polygon keeps (hostile files).
pub const MAX_WRAP_POINTS: usize = 1024;
/// Most points a derived outline has.
pub const MAX_DERIVED_POINTS: usize = 64;
/// How far outside the object a polygon point may lie (wrap space units, each way).
const MAX_COORD: i64 = WRAP_SPACE as i64 * 4;

/// A wrap polygon (`wp:wrapPolygon`): `wp:start` and the `wp:lineTo` points after it, closed.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[serde(default, rename_all = "camelCase")]
pub struct WrapPolygon {
    /// Edited by hand (`edited`) rather than worked out by the application.
    pub edited: bool,
    /// Points in the wrap space (x, y), the first being `wp:start`.
    pub points: Vec<[i32; 2]>,
}

impl WrapPolygon {
    /// A polygon from raw coordinates: clamped, capped at [`MAX_WRAP_POINTS`]. `None` with fewer
    /// than three points (no area).
    pub fn new(edited: bool, points: impl IntoIterator<Item = (i64, i64)>) -> Option<WrapPolygon> {
        let points: Vec<[i32; 2]> = points
            .into_iter()
            .take(MAX_WRAP_POINTS)
            .map(|(x, y)| [x.clamp(-MAX_COORD, MAX_COORD) as i32, y.clamp(-MAX_COORD, MAX_COORD) as i32])
            .collect();
        (points.len() >= 3).then_some(WrapPolygon { edited, points })
    }

    /// From an outline in unit coordinates (0..1 across the object).
    pub fn from_unit(edited: bool, pts: &[(f32, f32)]) -> Option<WrapPolygon> {
        let s = WRAP_SPACE as f32;
        WrapPolygon::new(
            edited,
            pts.iter().filter(|(x, y)| x.is_finite() && y.is_finite()).map(|(x, y)| ((x * s).round() as i64, (y * s).round() as i64)),
        )
    }

    /// The points in unit coordinates (0..1 across the object).
    pub fn unit(&self) -> Vec<(f32, f32)> {
        let s = WRAP_SPACE as f32;
        self.points.iter().take(MAX_WRAP_POINTS).map(|[x, y]| (*x as f32 / s, *y as f32 / s)).collect()
    }
}

/// A shape's outline in unit coordinates (0..1 across a `w` × `h` frame), flattened to straight
/// lines. `None` for shapes whose outline is their rectangle (rectangles, text boxes, lines,
/// freeforms). `corner` is a rounded rectangle's (see [`crate::connector::ShapeExtra::corner`]).
pub fn shape_outline(kind: ShapeKind, w: f32, h: f32, corner: Option<u32>) -> Option<Vec<(f32, f32)>> {
    use std::f32::consts::{FRAC_PI_2, PI, TAU};
    let pos = |v: f32| if v.is_finite() { v.clamp(0.0, 1.0e6) } else { 0.0 };
    let (w, h) = (pos(w), pos(h));
    let pts = match kind {
        // A freeform's paths (ink among them) aren't known from its kind: its frame is its outline.
        ShapeKind::Rectangle
        | ShapeKind::TextBox
        | ShapeKind::Line
        | ShapeKind::Freeform
        | ShapeKind::StraightConnector
        | ShapeKind::ElbowConnector
        | ShapeKind::CurvedConnector => return None,
        ShapeKind::Ellipse | ShapeKind::FlowchartOr => {
            (0..48).map(|i| (i as f32) * TAU / 48.0).map(|a| (0.5 + 0.5 * a.cos(), 0.5 + 0.5 * a.sin())).collect()
        }
        ShapeKind::RoundedRectangle => {
            // Corner radius as drawn.
            let r = crate::connector::corner_radius(corner, w, h);
            let (rx, ry) = if w > 0.0 && h > 0.0 { (r / w, r / h) } else { (0.0, 0.0) };
            let mut pts = Vec::new();
            for (cx, cy, a0) in [(1.0 - rx, 1.0 - ry, 0.0), (rx, 1.0 - ry, FRAC_PI_2), (rx, ry, PI), (1.0 - rx, ry, PI + FRAC_PI_2)] {
                for k in 0..=6 {
                    let a = a0 + FRAC_PI_2 * k as f32 / 6.0;
                    pts.push((cx + rx * a.cos(), cy + ry * a.sin()));
                }
            }
            pts
        }
        ShapeKind::Triangle => vec![(0.5, 0.0), (1.0, 1.0), (0.0, 1.0)],
        ShapeKind::RightTriangle => vec![(0.0, 0.0), (1.0, 1.0), (0.0, 1.0)],
        ShapeKind::Diamond => vec![(0.5, 0.0), (1.0, 0.5), (0.5, 1.0), (0.0, 0.5)],
        ShapeKind::Arrow | ShapeKind::ArrowLeft | ShapeKind::ArrowUp | ShapeKind::ArrowDown | ShapeKind::ArrowLeftRight | ShapeKind::ArrowUpDown => {
            arrow_outline(kind, w, h)
        }
        ShapeKind::Star
        | ShapeKind::Star4
        | ShapeKind::Star6
        | ShapeKind::Star7
        | ShapeKind::Star8
        | ShapeKind::Star10
        | ShapeKind::Star12
        | ShapeKind::Star16
        | ShapeKind::Star24
        | ShapeKind::Star32 => return star_outline(kind),
        ShapeKind::Heart => {
            let curves = [
                [(0.5, 0.3), (0.5, 0.0), (0.0, 0.0), (0.0, 0.3)],
                [(0.0, 0.3), (0.0, 0.6), (0.4, 0.75), (0.5, 1.0)],
                [(0.5, 1.0), (0.6, 0.75), (1.0, 0.6), (1.0, 0.3)],
                [(1.0, 0.3), (1.0, 0.0), (0.5, 0.0), (0.5, 0.3)],
            ];
            let mut pts = Vec::new();
            for [p0, p1, p2, p3] in curves {
                for k in 0..10 {
                    let t = k as f32 / 10.0;
                    let u = 1.0 - t;
                    let b = |a: f32, b: f32, c: f32, d: f32| u * u * u * a + 3.0 * u * u * t * b + 3.0 * u * t * t * c + t * t * t * d;
                    pts.push((b(p0.0, p1.0, p2.0, p3.0), b(p0.1, p1.1, p2.1, p3.1)));
                }
            }
            pts
        }
    };
    Some(pts)
}

/// A block arrow's outline in unit coordinates (0..1 across a `w` × `h` frame), pointing the way
/// `kind` says (right for anything but the left, up, down and double-headed arrows). Each head is
/// half the arrow's thickness long and never more than half its length, as the presets' `adj2`
/// of 50000 over `min(w, h)` gives (ECMA-376 Part 1 §20.1.10.56); the shaft is 40% of the
/// thickness thick.
pub fn arrow_outline(kind: ShapeKind, w: f32, h: f32) -> Vec<(f32, f32)> {
    let vertical = matches!(kind, ShapeKind::ArrowUp | ShapeKind::ArrowDown | ShapeKind::ArrowUpDown);
    let (len, thick) = if vertical { (h, w) } else { (w, h) };
    let head = if len > 0.0 { (0.5 * thick / len).min(0.5) } else { 0.5 };
    // Pointing right: `u` runs along the arrow from its tail, `v` across it. A double-headed
    // arrow has no tail: a head at each end of the shaft.
    let one = [(0.0, 0.3), (1.0 - head, 0.3), (1.0 - head, 0.0), (1.0, 0.5), (1.0 - head, 1.0), (1.0 - head, 0.7), (0.0, 0.7)];
    let two = [
        (0.0, 0.5),
        (head, 0.0),
        (head, 0.3),
        (1.0 - head, 0.3),
        (1.0 - head, 0.0),
        (1.0, 0.5),
        (1.0 - head, 1.0),
        (1.0 - head, 0.7),
        (head, 0.7),
        (head, 1.0),
    ];
    let pts: &[(f32, f32)] = if matches!(kind, ShapeKind::ArrowLeftRight | ShapeKind::ArrowUpDown) { &two } else { &one };
    pts.iter()
        .map(|&(u, v)| match kind {
            ShapeKind::ArrowLeft => (1.0 - u, v),
            ShapeKind::ArrowUp => (v, 1.0 - u),
            ShapeKind::ArrowDown | ShapeKind::ArrowUpDown => (v, u),
            _ => (u, v),
        })
        .collect()
}

/// A star preset's outline in unit coordinates (0..1 across its frame): one point at the top and
/// the rest evenly round it, with a notch between each pair ([`ShapeKind::star`] gives the counts
/// and the notch radius). `None` for a kind that is not a star.
///
/// The star is stretched to fill its frame, which is what the preset definitions' `hf` and `vf`
/// factors do (ECMA-376 Part 1 §20.1.10.56): for `star5` this gives Word's 1.05146 across and
/// 1.10557 down, and for `star6` its 1.15470 across.
pub fn star_outline(kind: ShapeKind) -> Option<Vec<(f32, f32)>> {
    use std::f32::consts::{FRAC_PI_2, PI};
    let (points, notch) = kind.star()?;
    // Both come from `ShapeKind::star`; the limits only keep the loop and the radius sane. The
    // most points a preset has is 32, which is exactly [`MAX_DERIVED_POINTS`] corners.
    let n = points.clamp(2, (MAX_DERIVED_POINTS / 2) as u32);
    let notch = if notch.is_finite() { notch.clamp(0.0, 1.0) } else { 0.5 };
    let pts: Vec<(f32, f32)> = (0..2 * n)
        .map(|i| {
            let a = PI * i as f32 / n as f32 - FRAC_PI_2;
            let r = if i % 2 == 0 { 1.0 } else { notch };
            (r * a.cos(), r * a.sin())
        })
        .collect();
    let bounds = |f: fn(&(f32, f32)) -> f32| {
        let (mut lo, mut hi) = (f32::INFINITY, f32::NEG_INFINITY);
        for p in &pts {
            lo = lo.min(f(p));
            hi = hi.max(f(p));
        }
        // A span of nothing (a notch radius of 1 on two points) leaves the star on its frame's edge.
        if lo.is_finite() && hi - lo > 1.0e-6 { (lo, hi - lo) } else { (0.0, 1.0) }
    };
    let ((x0, sw), (y0, sh)) = (bounds(|p| p.0), bounds(|p| p.1));
    Some(pts.iter().map(|(x, y)| (((x - x0) / sw).clamp(0.0, 1.0), ((y - y0) / sh).clamp(0.0, 1.0))).collect())
}

/// Largest side of the grid [`alpha_outline`] looks at.
pub const ALPHA_GRID: usize = 48;

/// The outline of a picture's opaque pixels, in unit coordinates: the convex hull of the cells
/// of a `gw` × `gh` grid laid over the picture that `opaque` says hold visible pixels, at most
/// [`MAX_DERIVED_POINTS`] points. `None` when every cell is opaque (or none is): the picture's
/// rectangle is its outline.
pub fn alpha_outline(gw: usize, gh: usize, opaque: impl Fn(usize, usize) -> bool) -> Option<Vec<(f32, f32)>> {
    let (gw, gh) = (gw.clamp(1, 1024), gh.clamp(1, 1024));
    // Each row's first and last opaque cells are all the hull can need.
    let mut pts: Vec<(i32, i32)> = Vec::new();
    let mut full = true;
    for y in 0..gh {
        let first = (0..gw).find(|&x| opaque(x, y));
        let Some(first) = first else {
            full = false;
            continue;
        };
        let last = (first..gw).rev().find(|&x| opaque(x, y)).unwrap_or(first);
        full &= first == 0 && last + 1 == gw;
        let (y0, y1) = (y as i32, y as i32 + 1);
        pts.extend([(first as i32, y0), (first as i32, y1), (last as i32 + 1, y0), (last as i32 + 1, y1)]);
    }
    if full || pts.is_empty() {
        return None;
    }
    let hull = simplify_convex(convex_hull(pts), MAX_DERIVED_POINTS);
    (hull.len() >= 3).then(|| hull.into_iter().map(|(x, y)| (x as f32 / gw as f32, y as f32 / gh as f32)).collect())
}

/// The convex hull of integer points, counter-clockwise (monotone chain).
fn convex_hull(mut pts: Vec<(i32, i32)>) -> Vec<(i32, i32)> {
    pts.sort_unstable();
    pts.dedup();
    if pts.len() < 3 {
        return pts;
    }
    let cross = |o: (i32, i32), a: (i32, i32), b: (i32, i32)| {
        (a.0 as i64 - o.0 as i64) * (b.1 as i64 - o.1 as i64) - (a.1 as i64 - o.1 as i64) * (b.0 as i64 - o.0 as i64)
    };
    let mut hull: Vec<(i32, i32)> = Vec::with_capacity(pts.len() * 2);
    for pass in 0..2 {
        let start = hull.len();
        let iter: Box<dyn Iterator<Item = &(i32, i32)>> = if pass == 0 { Box::new(pts.iter()) } else { Box::new(pts.iter().rev()) };
        for &p in iter {
            while hull.len() >= start + 2 {
                let (Some(&a), Some(&b)) = (hull.get(hull.len() - 2), hull.last()) else { break };
                if cross(a, b, p) <= 0 {
                    hull.pop();
                } else {
                    break;
                }
            }
            hull.push(p);
        }
        hull.pop();
    }
    hull
}

/// A convex polygon cut down to `max` points by dropping, one at a time, the corner whose
/// removal loses the least area.
fn simplify_convex(mut pts: Vec<(i32, i32)>, max: usize) -> Vec<(i32, i32)> {
    while pts.len() > max.max(3) {
        let n = pts.len();
        let area = |i: usize| {
            let (a, b, c) = (pts[(i + n - 1) % n], pts[i], pts[(i + 1) % n]);
            ((b.0 as i64 - a.0 as i64) * (c.1 as i64 - a.1 as i64) - (b.1 as i64 - a.1 as i64) * (c.0 as i64 - a.0 as i64)).abs()
        };
        let Some(i) = (0..n).min_by_key(|&i| area(i)) else { break };
        pts.remove(i);
    }
    pts
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn polygon_caps_and_clamps() {
        assert!(WrapPolygon::new(false, [(0, 0), (1, 1)]).is_none());
        let p = WrapPolygon::new(true, (0..5000).map(|i| (i, i64::MAX))).unwrap();
        assert_eq!(p.points.len(), MAX_WRAP_POINTS);
        assert!(p.points.iter().all(|[_, y]| *y as i64 == MAX_COORD));
        assert!(WrapPolygon::from_unit(false, &[(f32::NAN, 0.0), (0.0, 0.0), (1.0, 0.0), (0.5, 1.0)]).is_some_and(|p| p.points.len() == 3));
    }

    /// #3919: the tip of each block arrow lies on the side it points to.
    #[test]
    fn arrow_outlines_point_the_right_way() {
        let tip = |kind| shape_outline(kind, 100.0, 100.0, None).unwrap()[3];
        assert_eq!(tip(ShapeKind::Arrow), (1.0, 0.5));
        assert_eq!(tip(ShapeKind::ArrowLeft), (0.0, 0.5));
        assert_eq!(tip(ShapeKind::ArrowUp), (0.5, 0.0));
        assert_eq!(tip(ShapeKind::ArrowDown), (0.5, 1.0));
    }

    /// #6391: each star preset draws its own number of points, with the notch radius Word's
    /// default `adj` gives, stretched to fill its frame as the presets' `hf` and `vf` do.
    #[test]
    fn star_outlines_keep_their_point_count() {
        for kind in ShapeKind::STARS {
            let (points, notch) = kind.star().unwrap_or((0, 0.0));
            let pts = shape_outline(kind, 100.0, 60.0, None).unwrap();
            assert_eq!(pts.len(), 2 * points as usize, "{kind:?} draws {points} points");
            // One point at the top, and the star reaches all four sides of its frame.
            assert_eq!(pts.first().map(|(x, y)| ((x * 1000.0).round(), *y)), Some((500.0, 0.0)), "{kind:?}");
            let span = |f: fn(&(f32, f32)) -> f32| pts.iter().fold((1.0f32, 0.0f32), |(lo, hi), p| (lo.min(f(p)), hi.max(f(p))));
            assert_eq!(span(|p| p.0), (0.0, 1.0), "{kind:?} fills its frame across");
            assert_eq!(span(|p| p.1), (0.0, 1.0), "{kind:?} fills its frame down");
            // In a square frame, a star whose own bounds are square (every one but the 5, 6, 7
            // and 10 point stars) keeps its radii: the points reach the frame, the notches the
            // preset's fraction of it.
            if matches!(kind, ShapeKind::Star4 | ShapeKind::Star8 | ShapeKind::Star12 | ShapeKind::Star16 | ShapeKind::Star24 | ShapeKind::Star32) {
                let square = shape_outline(kind, 80.0, 80.0, None).unwrap();
                let radius = |i: usize| square.get(i).map_or(0.0, |(x, y)| 2.0 * (x - 0.5f32).hypot(y - 0.5));
                for i in 0..square.len() {
                    let want = if i % 2 == 0 { 1.0 } else { notch };
                    assert!((radius(i) - want).abs() < 0.001, "{kind:?} point {i}: radius {} is not {want}", radius(i));
                }
            }
        }
        // The five-point star is pinned to Word's own proportions: `hf` 1.05146 across and `vf`
        // 1.10557 down put its right-hand point on the frame's edge, 38.2% of the way down, and
        // its lower points on the bottom edge.
        let five = shape_outline(ShapeKind::Star, 90.0, 90.0, None).unwrap();
        let at = |i: usize| five.get(i).copied().unwrap_or_default();
        assert!((at(2).0 - 1.0).abs() < 0.001 && (at(2).1 - 0.381_97).abs() < 0.001, "{:?}", at(2));
        assert!((at(4).0 - 0.809_02).abs() < 0.001 && (at(4).1 - 1.0).abs() < 0.001, "{:?}", at(4));
        // A six-point star has a point at the top and the bottom, and reaches the sides between.
        let six = shape_outline(ShapeKind::Star6, 90.0, 90.0, None).unwrap();
        assert_eq!(six.len(), 12);
        assert!((six[6].0 - 0.5).abs() < 0.001 && (six[6].1 - 1.0).abs() < 0.001, "{:?}", six[6]);
        // An eight-point star has more corners than a five-point one: the regression this fixes.
        assert!(shape_outline(ShapeKind::Star8, 90.0, 90.0, None).unwrap().len() > five.len());
        // Junk sizes and counts never panic, and give an outline inside the frame.
        for (w, h) in [(0.0, 0.0), (f32::NAN, 10.0), (-5.0, f32::INFINITY), (1.0e12, 0.5)] {
            for kind in ShapeKind::STARS {
                let pts = shape_outline(kind, w, h, None).unwrap_or_default();
                assert!(pts.iter().all(|(x, y)| (0.0..=1.0).contains(x) && (0.0..=1.0).contains(y)), "{kind:?} {w}x{h}: {pts:?}");
            }
        }
        assert_eq!(star_outline(ShapeKind::Heart), None);
        assert_eq!(star_outline(ShapeKind::Rectangle), None);
    }

    /// #6391: `leftRightArrow` and `upDownArrow` have a head at each end, where both used to draw
    /// as the single right-pointing arrow (and, in a tall frame, as a triangle).
    #[test]
    fn double_headed_arrows_have_a_head_at_each_end() {
        // 100 × 40: each head is half the thickness long, a fifth of the length.
        let wide = shape_outline(ShapeKind::ArrowLeftRight, 100.0, 40.0, None).unwrap();
        assert_eq!(wide, [(0.0, 0.5), (0.2, 0.0), (0.2, 0.3), (0.8, 0.3), (0.8, 0.0), (1.0, 0.5), (0.8, 1.0), (0.8, 0.7), (0.2, 0.7), (0.2, 1.0)]);
        // Turned a quarter: the same arrow pointing up and down.
        let tall = shape_outline(ShapeKind::ArrowUpDown, 40.0, 100.0, None).unwrap();
        assert_eq!(tall, wide.iter().map(|&(u, v)| (v, u)).collect::<Vec<_>>());
        // A head is never longer than half the arrow, so the two never cross in a square frame.
        for (w, h) in [(40.0, 40.0), (20.0, 100.0), (100.0, 20.0), (0.0, 0.0), (f32::NAN, 5.0)] {
            for kind in [ShapeKind::ArrowLeftRight, ShapeKind::ArrowUpDown] {
                let pts = shape_outline(kind, w, h, None).unwrap();
                assert_eq!(pts.len(), 10, "{kind:?} {w}x{h}");
                assert!(pts.iter().all(|(x, y)| (0.0..=1.0).contains(x) && (0.0..=1.0).contains(y)), "{kind:?} {w}x{h}: {pts:?}");
                let (lo, hi) = pts.iter().fold((1.0f32, 0.0f32), |(lo, hi), p| (lo.min(p.0.min(p.1)), hi.max(p.0.max(p.1))));
                assert!(lo == 0.0 && hi == 1.0, "{kind:?} {w}x{h} fills its frame");
            }
        }
        // The single arrows are untouched: one head, a tail across the whole shaft.
        let right = shape_outline(ShapeKind::Arrow, 100.0, 40.0, None).unwrap();
        assert_eq!(right, [(0.0, 0.3), (0.8, 0.3), (0.8, 0.0), (1.0, 0.5), (0.8, 1.0), (0.8, 0.7), (0.0, 0.7)]);
    }

    #[test]
    fn shape_outlines_stay_in_the_frame() {
        for kind in [
            ShapeKind::Ellipse,
            ShapeKind::RoundedRectangle,
            ShapeKind::Triangle,
            ShapeKind::RightTriangle,
            ShapeKind::Diamond,
            ShapeKind::Arrow,
            ShapeKind::ArrowLeft,
            ShapeKind::ArrowUp,
            ShapeKind::ArrowDown,
            ShapeKind::ArrowLeftRight,
            ShapeKind::ArrowUpDown,
            ShapeKind::Star,
            ShapeKind::Star4,
            ShapeKind::Star7,
            ShapeKind::Star32,
            ShapeKind::Heart,
        ] {
            for (w, h) in [(100.0, 50.0), (0.0, 0.0), (f32::NAN, 10.0)] {
                for corner in [None, Some(0), Some(3_565), Some(u32::MAX)] {
                    let pts = shape_outline(kind, w, h, corner).unwrap();
                    assert!(pts.len() >= 3 && pts.len() <= MAX_DERIVED_POINTS, "{kind:?}");
                    assert!(pts.iter().all(|(x, y)| (-0.001..=1.001).contains(x) && (-0.001..=1.001).contains(y)), "{kind:?}");
                }
            }
        }
        assert!(shape_outline(ShapeKind::Rectangle, 10.0, 10.0, None).is_none());
        // The end of the bottom-right corner: in the frame's corner when the corner is square,
        // 16% of the height (8 of 100 wide) in from it by default.
        let corner_pt = |corner| shape_outline(ShapeKind::RoundedRectangle, 100.0, 50.0, corner).unwrap()[6];
        assert!(corner_pt(Some(0)).0 > 0.999 && corner_pt(Some(0)).1 > 0.999, "{:?}", corner_pt(Some(0)));
        assert!((corner_pt(None).0 - 0.92).abs() < 0.001, "{:?}", corner_pt(None));
    }

    #[test]
    fn alpha_outline_is_the_hull_of_opaque_cells() {
        // A disc in a 20×20 grid.
        let disc = |x: usize, y: usize| {
            let (dx, dy) = (x as f32 + 0.5 - 10.0, y as f32 + 0.5 - 10.0);
            dx * dx + dy * dy < 64.0
        };
        let pts = alpha_outline(20, 20, disc).unwrap();
        assert!(pts.len() >= 8 && pts.len() <= MAX_DERIVED_POINTS);
        assert!(pts.iter().all(|(x, y)| (0.05..=0.95).contains(x) && (0.05..=0.95).contains(y)));
        assert!(alpha_outline(20, 20, |_, _| true).is_none());
        assert!(alpha_outline(20, 20, |_, _| false).is_none());
        // A big grid with a noisy outline is cut down to the cap.
        let noisy = alpha_outline(1024, 1024, |x, y| {
            let (dx, dy) = (x as f32 - 512.0, y as f32 - 512.0);
            dx * dx + dy * dy < 500.0 * 500.0
        })
        .unwrap();
        assert!(noisy.len() <= MAX_DERIVED_POINTS);
    }
}
