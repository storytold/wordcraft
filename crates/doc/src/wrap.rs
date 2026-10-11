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
/// freeforms).
pub fn shape_outline(kind: ShapeKind, w: f32, h: f32) -> Option<Vec<(f32, f32)>> {
    use std::f32::consts::{FRAC_PI_2, PI, TAU};
    let pos = |v: f32| if v.is_finite() { v.clamp(0.0, 1.0e6) } else { 0.0 };
    let (w, h) = (pos(w), pos(h));
    let pts = match kind {
        // A freeform's paths (ink among them) aren't known from its kind: its frame is its outline.
        ShapeKind::Rectangle | ShapeKind::TextBox | ShapeKind::Line | ShapeKind::Freeform => return None,
        ShapeKind::Ellipse => (0..48).map(|i| (i as f32) * TAU / 48.0).map(|a| (0.5 + 0.5 * a.cos(), 0.5 + 0.5 * a.sin())).collect(),
        ShapeKind::RoundedRectangle => {
            // Corner radius as drawn: 16% of the shorter side.
            let r = 0.16 * w.min(h);
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
        ShapeKind::Diamond => vec![(0.5, 0.0), (1.0, 0.5), (0.5, 1.0), (0.0, 0.5)],
        ShapeKind::Arrow => {
            // As drawn: the head is half the height long, the shaft 40% of the height thick.
            let head = if w > 0.0 { (0.5 * h / w).min(1.0) } else { 0.5 };
            vec![(0.0, 0.3), (1.0 - head, 0.3), (1.0 - head, 0.0), (1.0, 0.5), (1.0 - head, 1.0), (1.0 - head, 0.7), (0.0, 0.7)]
        }
        ShapeKind::Star => (0..10)
            .map(|i| {
                let a = PI * i as f32 / 5.0 - FRAC_PI_2;
                let r = if i % 2 == 0 { 0.5 } else { 0.2 };
                (0.5 + r * a.cos(), 0.5 + r * a.sin())
            })
            .collect(),
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

    #[test]
    fn shape_outlines_stay_in_the_frame() {
        for kind in [
            ShapeKind::Ellipse,
            ShapeKind::RoundedRectangle,
            ShapeKind::Triangle,
            ShapeKind::Diamond,
            ShapeKind::Arrow,
            ShapeKind::Star,
            ShapeKind::Heart,
        ] {
            for (w, h) in [(100.0, 50.0), (0.0, 0.0), (f32::NAN, 10.0)] {
                let pts = shape_outline(kind, w, h).unwrap();
                assert!(pts.len() >= 3 && pts.len() <= MAX_DERIVED_POINTS, "{kind:?}");
                assert!(pts.iter().all(|(x, y)| (-0.001..=1.001).contains(x) && (-0.001..=1.001).contains(y)), "{kind:?}");
            }
        }
        assert!(shape_outline(ShapeKind::Rectangle, 10.0, 10.0).is_none());
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
