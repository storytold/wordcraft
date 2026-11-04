//! Alignment guides: while a picture, shape or text box is dragged, its left, centre or right
//! edge (and top, middle or bottom) snaps to the page's edges and centre, the margins and the
//! edges and centres of the other floating objects on its page, and a thin line across the page
//! shows each place it snapped to. WordCraft's own design (Word for Mac shows no guides).
//!
//! The maths here is pure (points in, points out) so it's tested without egui; `objects.rs`
//! gathers the candidates and draws the lines.

use wordcraft_geom::Rect;

/// How near (screen points) an edge or centre must come to a guide to snap to it.
pub const SNAP_SCREEN: f32 = 6.0;

/// Which way a guide runs: `X` is a vertical line at `at` (lefts, centres and rights snap to it),
/// `Y` a horizontal one (tops, middles and bottoms).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Axis {
    X,
    Y,
}

/// A place to snap to on the page, points.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Guide {
    pub axis: Axis,
    pub at: f32,
}

impl Guide {
    pub fn x(at: f32) -> Guide {
        Guide { axis: Axis::X, at }
    }
    pub fn y(at: f32) -> Guide {
        Guide { axis: Axis::Y, at }
    }
}

/// What snapping does to a dragged rect: move it by (`dx`, `dy`) points, and show `lines`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Snap {
    pub dx: f32,
    pub dy: f32,
    pub lines: Vec<Guide>,
}

/// Snap `moving` (the dragged object's bounds, points) to the nearest of `candidates` within
/// `threshold` points, x and y independently: its left, centre or right to an `X` guide, its top,
/// middle or bottom to a `Y` guide. Nearest wins; on a tie, the first candidate. The lines are
/// every guide an edge or centre sits on once moved.
pub fn snap(moving: Rect, candidates: &[Guide], threshold: f32) -> Snap {
    if !threshold.is_finite() || threshold < 0.0 {
        return Snap::default();
    }
    let xs = [moving.x, moving.x + moving.w / 2.0, moving.x + moving.w];
    let ys = [moving.y, moving.y + moving.h / 2.0, moving.y + moving.h];
    let mut out = Snap::default();
    for (axis, points) in [(Axis::X, xs), (Axis::Y, ys)] {
        let Some(d) = nearest(&points, candidates, axis, threshold) else { continue };
        match axis {
            Axis::X => out.dx = d,
            Axis::Y => out.dy = d,
        }
        for g in candidates.iter().filter(|g| g.axis == axis && g.at.is_finite()) {
            if points.iter().any(|p| (p + d - g.at).abs() <= LANDED) && !out.lines.contains(g) {
                out.lines.push(*g);
            }
        }
    }
    out
}

/// How near (points) an edge must be to a guide after snapping to count as on it.
const LANDED: f32 = 0.01;

/// The shortest move (within `threshold`) that puts one of `points` on an `axis` guide.
fn nearest(points: &[f32], candidates: &[Guide], axis: Axis, threshold: f32) -> Option<f32> {
    let mut best: Option<f32> = None;
    for g in candidates.iter().filter(|g| g.axis == axis && g.at.is_finite()) {
        for p in points.iter().filter(|p| p.is_finite()) {
            let d = g.at - p;
            if d.abs() <= threshold && best.is_none_or(|b| d.abs() < b.abs()) {
                best = Some(d);
            }
        }
    }
    best
}

/// The guides of a page `w` × `h` points with text area `body`, and of the other floating objects
/// on it (their bounds): edges and centres.
pub fn candidates(w: f32, h: f32, body: Rect, others: impl IntoIterator<Item = Rect>) -> Vec<Guide> {
    let mut g = Vec::new();
    let mut add = |r: Rect| {
        for x in [r.x, r.x + r.w / 2.0, r.x + r.w] {
            g.push(Guide::x(x));
        }
        for y in [r.y, r.y + r.h / 2.0, r.y + r.h] {
            g.push(Guide::y(y));
        }
    };
    add(Rect::new(0.0, 0.0, w, h));
    add(body);
    for r in others {
        add(r);
    }
    g.retain(|g| g.at.is_finite());
    g
}

#[cfg(test)]
mod tests {
    use super::*;

    const R: Rect = Rect { x: 100.0, y: 200.0, w: 50.0, h: 20.0 };

    #[test]
    fn edges_snap_to_guides() {
        // Left edge 3 pt from x = 97: moves left onto it.
        let s = snap(R, &[Guide::x(97.0)], 6.0);
        assert_eq!((s.dx, s.dy), (-3.0, 0.0));
        assert_eq!(s.lines, vec![Guide::x(97.0)]);
        // Right edge (150) to 154.
        let s = snap(R, &[Guide::x(154.0)], 6.0);
        assert_eq!(s.dx, 4.0);
        // Bottom (220) to 218.
        let s = snap(R, &[Guide::y(218.0)], 6.0);
        assert_eq!((s.dx, s.dy), (0.0, -2.0));
        assert_eq!(s.lines, vec![Guide::y(218.0)]);
    }

    #[test]
    fn centres_snap_to_guides() {
        // Centre 125, middle 210.
        let s = snap(R, &[Guide::x(127.0), Guide::y(207.5)], 6.0);
        assert_eq!((s.dx, s.dy), (2.0, -2.5));
        assert_eq!(s.lines, vec![Guide::x(127.0), Guide::y(207.5)]);
    }

    #[test]
    fn the_nearest_guide_wins() {
        // Left (100) is 5 from 95; right (150) is 1 from 151: the right edge snaps.
        let s = snap(R, &[Guide::x(95.0), Guide::x(151.0)], 6.0);
        assert_eq!(s.dx, 1.0);
        assert_eq!(s.lines, vec![Guide::x(151.0)]);
    }

    #[test]
    fn guides_beyond_the_threshold_are_ignored() {
        let s = snap(R, &[Guide::x(93.0), Guide::y(230.0)], 6.0);
        assert_eq!(s, Snap::default());
        // Exactly at the threshold still snaps.
        assert_eq!(snap(R, &[Guide::x(94.0)], 6.0).dx, -6.0);
    }

    #[test]
    fn x_and_y_snap_independently() {
        // Only y has a guide in reach: x stays put.
        let s = snap(R, &[Guide::x(300.0), Guide::y(199.0)], 6.0);
        assert_eq!((s.dx, s.dy), (0.0, -1.0));
        assert_eq!(s.lines, vec![Guide::y(199.0)]);
    }

    #[test]
    fn every_guide_an_edge_lands_on_is_shown() {
        // Snapping the left edge to 98 also puts the right edge on 148.
        let s = snap(R, &[Guide::x(98.0), Guide::x(148.0), Guide::x(148.0)], 6.0);
        assert_eq!(s.dx, -2.0);
        assert_eq!(s.lines, vec![Guide::x(98.0), Guide::x(148.0)], "each line once");
    }

    #[test]
    fn nothing_to_snap_to() {
        assert_eq!(snap(R, &[], 6.0), Snap::default());
        // Junk never moves anything.
        let junk = [Guide::x(f32::NAN), Guide::y(f32::INFINITY)];
        assert_eq!(snap(R, &junk, 6.0), Snap::default());
        assert_eq!(snap(R, &[Guide::x(99.0)], f32::NAN), Snap::default());
        assert_eq!(snap(R, &[Guide::x(99.0)], -1.0), Snap::default());
        assert_eq!(snap(Rect::new(f32::NAN, 0.0, 1.0, 1.0), &[Guide::x(0.0)], 6.0).dx, 0.0);
    }

    #[test]
    fn candidates_are_page_margins_and_other_objects() {
        let body = Rect::new(72.0, 72.0, 468.0, 648.0);
        let g = candidates(612.0, 792.0, body, [Rect::new(10.0, 20.0, 30.0, 40.0)]);
        for x in [0.0, 306.0, 612.0, 72.0, 540.0, 10.0, 25.0, 40.0] {
            assert!(g.contains(&Guide::x(x)), "x {x}: {g:?}");
        }
        for y in [0.0, 396.0, 792.0, 72.0, 720.0, 20.0, 40.0, 60.0] {
            assert!(g.contains(&Guide::y(y)), "y {y}: {g:?}");
        }
    }
}
