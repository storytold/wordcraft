//! The outlines of the polygonal preset shapes (hexagon, octagon, pentagon, parallelogram,
//! trapezoid, plus, chevron, decagon), from the DrawingML preset shape definitions (ECMA-376
//! Part 1 §20.1.10.56, `presetShapeDefinitions.xml`) with their default adjust values. The
//! raster renderer, the PDF writer and text wrapping all draw these from here.

use crate::para::ShapeKind;

/// The outline of a polygonal preset `kind` in a `w` × `h` box: its corners in drawing order, in
/// unit coordinates (0..1 across the box). `None` for the other kinds. Corners that the presets
/// set from the shorter side (a hexagon's, an octagon's…) stay that far in whatever the box's
/// shape; a zero or junk size gives a flat outline, still inside the unit square.
pub fn polygon(kind: ShapeKind, w: f32, h: f32) -> Option<Vec<(f32, f32)>> {
    let pos = |v: f32| if v.is_finite() { v.clamp(0.0, 1.0e6) } else { 0.0 };
    let (w, h) = (pos(w), pos(h));
    let ss = w.min(h);
    // A distance across (`fx`) or down (`fy`) the box as a fraction of it.
    let fx = |d: f32| if w > 0.0 { (d / w).clamp(0.0, 1.0) } else { 0.0 };
    let fy = |d: f32| if h > 0.0 { (d / h).clamp(0.0, 1.0) } else { 0.0 };
    let (sin, cos) = (|deg: f32| deg.to_radians().sin(), |deg: f32| deg.to_radians().cos());
    let pts = match kind {
        // `adj` 25000 (a quarter of the shorter side), `vf` 115470: the slanted sides meet the
        // top and bottom.
        ShapeKind::Hexagon => {
            let x = fx(ss * 0.25).min(0.5);
            let dy = 0.5 * 1.154_70 * sin(60.0);
            vec![(0.0, 0.5), (x, 0.5 - dy), (1.0 - x, 0.5 - dy), (1.0, 0.5), (1.0 - x, 0.5 + dy), (x, 0.5 + dy)]
        }
        // `adj` 29289: the corners cut 29.289% of the shorter side, which makes a square's regular.
        ShapeKind::Octagon => {
            let (x, y) = (fx(ss * 0.292_89).min(0.5), fy(ss * 0.292_89).min(0.5));
            vec![(0.0, y), (x, 0.0), (1.0 - x, 0.0), (1.0, y), (1.0, 1.0 - y), (1.0 - x, 1.0), (x, 1.0), (0.0, 1.0 - y)]
        }
        // `hf` 105146, `vf` 110557 stretch a regular pentagon to fill the box.
        ShapeKind::Pentagon => {
            let (swd2, shd2) = (0.5 * 1.051_46, 0.5 * 1.105_57);
            let svc = shd2;
            let (dx1, dx2) = (swd2 * cos(18.0), swd2 * cos(306.0));
            let (y1, y2) = (svc - shd2 * sin(18.0), svc - shd2 * sin(306.0));
            vec![(0.5 - dx1, y1), (0.5, 0.0), (0.5 + dx1, y1), (0.5 + dx2, y2), (0.5 - dx2, y2)]
        }
        // `adj` 25000: the top slides a quarter of the shorter side right.
        ShapeKind::Parallelogram => {
            let x = fx(ss * 0.25);
            vec![(0.0, 1.0), (x, 0.0), (1.0, 0.0), (1.0 - x, 1.0)]
        }
        // `adj` 25000: the top is a quarter of the shorter side in at each end.
        ShapeKind::Trapezoid => {
            let x = fx(ss * 0.25).min(0.5);
            vec![(0.0, 1.0), (x, 0.0), (1.0 - x, 0.0), (1.0, 1.0)]
        }
        // `adj` 25000: the arms are a quarter of the shorter side in from each side.
        ShapeKind::Plus => {
            let (x, y) = (fx(ss * 0.25).min(0.5), fy(ss * 0.25).min(0.5));
            vec![
                (0.0, y),
                (x, y),
                (x, 0.0),
                (1.0 - x, 0.0),
                (1.0 - x, y),
                (1.0, y),
                (1.0, 1.0 - y),
                (1.0 - x, 1.0 - y),
                (1.0 - x, 1.0),
                (x, 1.0),
                (x, 1.0 - y),
                (0.0, 1.0 - y),
            ]
        }
        // `adj` 50000: the point and the notch are half the shorter side deep.
        ShapeKind::Chevron => {
            let x = fx(ss * 0.5);
            vec![(0.0, 0.0), (1.0 - x, 0.0), (1.0, 0.5), (1.0 - x, 1.0), (0.0, 1.0), (x, 0.5)]
        }
        // `vf` 105146 stretches a regular decagon to fill the box's height.
        ShapeKind::Decagon => {
            let shd2 = 0.5 * 1.051_46;
            let (dx1, dx2) = (0.5 * cos(36.0), 0.5 * cos(72.0));
            let (dy1, dy2) = (shd2 * sin(72.0), shd2 * sin(36.0));
            vec![
                (0.0, 0.5),
                (0.5 - dx1, 0.5 - dy2),
                (0.5 - dx2, 0.5 - dy1),
                (0.5 + dx2, 0.5 - dy1),
                (0.5 + dx1, 0.5 - dy2),
                (1.0, 0.5),
                (0.5 + dx1, 0.5 + dy2),
                (0.5 + dx2, 0.5 + dy1),
                (0.5 - dx2, 0.5 + dy1),
                (0.5 - dx1, 0.5 + dy2),
            ]
        }
        _ => return None,
    };
    // The presets' constants put some corners a hair past the box's edge.
    Some(pts.into_iter().map(|(x, y)| (x.clamp(0.0, 1.0), y.clamp(0.0, 1.0))).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    const NEW: [(ShapeKind, usize); 8] = [
        (ShapeKind::Hexagon, 6),
        (ShapeKind::Octagon, 8),
        (ShapeKind::Pentagon, 5),
        (ShapeKind::Parallelogram, 4),
        (ShapeKind::Trapezoid, 4),
        (ShapeKind::Plus, 12),
        (ShapeKind::Chevron, 6),
        (ShapeKind::Decagon, 10),
    ];

    #[test]
    fn presets_are_closed_outlines_inside_their_box() {
        for (kind, n) in NEW {
            for (w, h) in [(100.0, 60.0), (60.0, 100.0), (10.0, 10.0)] {
                let pts = polygon(kind, w, h).unwrap_or_else(|| panic!("{kind:?} has an outline"));
                assert_eq!(pts.len(), n, "{kind:?} {w}×{h}");
                assert!(pts.iter().all(|(x, y)| (0.0..=1.0).contains(x) && (0.0..=1.0).contains(y)), "{kind:?} {w}×{h}: {pts:?}");
                // It reaches all four sides of its box.
                let (min_x, max_x) = pts.iter().fold((1.0f32, 0.0f32), |(a, b), (x, _)| (a.min(*x), b.max(*x)));
                let (min_y, max_y) = pts.iter().fold((1.0f32, 0.0f32), |(a, b), (_, y)| (a.min(*y), b.max(*y)));
                assert!(min_x < 0.001 && max_x > 0.999 && min_y < 0.001 && max_y > 0.999, "{kind:?} {w}×{h}: {pts:?}");
            }
            // Degenerate boxes: still inside the unit square, no panic.
            for (w, h) in [(0.0, 0.0), (0.0, 50.0), (50.0, 0.0), (f32::NAN, 10.0), (f32::INFINITY, -1.0), (-5.0, 5.0)] {
                let pts = polygon(kind, w, h).unwrap_or_default();
                assert!(pts.iter().all(|(x, y)| (0.0..=1.0).contains(x) && (0.0..=1.0).contains(y)), "{kind:?} {w}×{h}: {pts:?}");
            }
        }
        // Other kinds have their own outlines.
        assert!(polygon(ShapeKind::Rectangle, 10.0, 10.0).is_none());
        assert!(polygon(ShapeKind::Heart, 10.0, 10.0).is_none());
    }

    #[test]
    fn presets_follow_their_default_adjust_values() {
        // A hexagon 100 × 60: its corners are a quarter of the shorter side (15) in from the ends.
        let hex = polygon(ShapeKind::Hexagon, 100.0, 60.0).unwrap();
        assert!((hex[1].0 - 0.15).abs() < 0.001 && hex[1].1.abs() < 0.001, "{hex:?}");
        // An octagon's cut is 29.289% of the shorter side.
        let oct = polygon(ShapeKind::Octagon, 100.0, 100.0).unwrap();
        assert!((oct[1].0 - 0.29289).abs() < 0.001, "{oct:?}");
        // A plus's arms are a quarter of the shorter side in.
        let plus = polygon(ShapeKind::Plus, 100.0, 100.0).unwrap();
        assert!((plus[0].1 - 0.25).abs() < 0.001, "{plus:?}");
        // A regular pentagon's top point is at the middle, its sides' ends 38.2% down.
        let pent = polygon(ShapeKind::Pentagon, 100.0, 100.0).unwrap();
        assert!((pent[1].0 - 0.5).abs() < 0.001 && pent[1].1.abs() < 0.001, "{pent:?}");
        assert!((pent[0].1 - 0.382).abs() < 0.002, "{pent:?}");
        // A chevron's point is half the shorter side deep, at the right middle.
        let chev = polygon(ShapeKind::Chevron, 100.0, 60.0).unwrap();
        assert!((chev[1].0 - 0.7).abs() < 0.001 && (chev[2].0 - 1.0).abs() < 0.001 && (chev[2].1 - 0.5).abs() < 0.001, "{chev:?}");
        // A trapezoid is narrow at the top.
        let trap = polygon(ShapeKind::Trapezoid, 100.0, 60.0).unwrap();
        assert!((trap[1].0 - 0.15).abs() < 0.001 && trap[1].1.abs() < 0.001, "{trap:?}");
    }
}
