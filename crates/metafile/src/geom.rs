//! Path operations in `f64` and the shape builders that turn WMF/EMF rectangles, ellipses and arcs
//! into cubic Béziers (the classic 4/3·tan(Δ/4) approximation, at most 90° per piece).

use std::f64::consts::{FRAC_PI_2, PI, TAU};

pub(crate) type P = (f64, f64);

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum Op {
    M(P),
    L(P),
    C(P, P, P),
    Z,
}

pub(crate) fn rect(l: f64, t: f64, r: f64, b: f64) -> Vec<Op> {
    vec![Op::M((l, t)), Op::L((r, t)), Op::L((r, b)), Op::L((l, b)), Op::Z]
}

/// Corner radii are half the EMF/WMF corner ellipse size, clamped so the corners never overlap.
pub(crate) fn round_rect(l: f64, t: f64, r: f64, b: f64, ew: f64, eh: f64) -> Vec<Op> {
    // Reversed corners (right < left, bottom < top) describe the same rectangle.
    let (l, r) = (l.min(r), l.max(r));
    let (t, b) = (t.min(b), t.max(b));
    let rx = (ew / 2.0).abs().min((r - l) / 2.0);
    let ry = (eh / 2.0).abs().min((b - t) / 2.0);
    let mut ops = vec![Op::M((l + rx, t)), Op::L((r - rx, t))];
    ops.extend(arc(r - rx, t + ry, rx, ry, -FRAC_PI_2, FRAC_PI_2));
    ops.push(Op::L((r, b - ry)));
    ops.extend(arc(r - rx, b - ry, rx, ry, 0.0, FRAC_PI_2));
    ops.push(Op::L((l + rx, b)));
    ops.extend(arc(l + rx, b - ry, rx, ry, FRAC_PI_2, FRAC_PI_2));
    ops.push(Op::L((l, t + ry)));
    ops.extend(arc(l + rx, t + ry, rx, ry, PI, FRAC_PI_2));
    ops.push(Op::Z);
    ops
}

pub(crate) fn ellipse(l: f64, t: f64, r: f64, b: f64) -> Vec<Op> {
    let (cx, cy, rx, ry) = center(l, t, r, b);
    let mut ops = vec![Op::M((cx + rx, cy))];
    ops.extend(arc(cx, cy, rx, ry, 0.0, TAU));
    ops.push(Op::Z);
    ops
}

/// Which closed or open shape an ARC/CHORD/PIE record draws.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum ArcKind {
    Arc,
    Chord,
    Pie,
}

/// ARC, CHORD and PIE: the arc runs counter-clockwise on screen from the start point to the end point.
pub(crate) fn arc_shape(l: f64, t: f64, r: f64, b: f64, s: P, e: P, kind: ArcKind) -> Vec<Op> {
    let (cx, cy, rx, ry) = center(l, t, r, b);
    let ang = |p: P| ((p.1 - cy) / ry.max(f64::MIN_POSITIVE)).atan2((p.0 - cx) / rx.max(f64::MIN_POSITIVE));
    let (a0, a1) = (ang(s), ang(e));
    let mut d = (a0 - a1).rem_euclid(TAU);
    if d < 1e-12 {
        d = TAU;
    }
    let sweep = -d;
    let start = (cx + rx * a0.cos(), cy + ry * a0.sin());
    let mut ops = if kind == ArcKind::Pie { vec![Op::M((cx, cy)), Op::L(start)] } else { vec![Op::M(start)] };
    ops.extend(arc(cx, cy, rx, ry, a0, sweep));
    match kind {
        ArcKind::Arc => {}
        ArcKind::Chord => ops.push(Op::Z),
        ArcKind::Pie => ops.extend([Op::L((cx, cy)), Op::Z]),
    }
    ops
}

fn center(l: f64, t: f64, r: f64, b: f64) -> (f64, f64, f64, f64) {
    ((l + r) / 2.0, (t + b) / 2.0, (r - l).abs() / 2.0, (b - t).abs() / 2.0)
}

/// Cubic pieces along an ellipse from parameter `a0` over signed `sweep` radians. The current point
/// must already be at `(cx + rx cos a0, cy + ry sin a0)`.
fn arc(cx: f64, cy: f64, rx: f64, ry: f64, a0: f64, sweep: f64) -> Vec<Op> {
    let n = ((sweep.abs() / FRAC_PI_2).ceil() as usize).clamp(1, 8);
    let step = sweep / n as f64;
    let k = 4.0 / 3.0 * (step / 4.0).tan();
    let pt = |t: f64| (cx + rx * t.cos(), cy + ry * t.sin());
    let mut ops = Vec::with_capacity(n);
    for i in 0..n {
        let t0 = a0 + step * i as f64;
        let t1 = t0 + step;
        let (p0, p3) = (pt(t0), pt(t1));
        let c1 = (p0.0 - k * rx * t0.sin(), p0.1 + k * ry * t0.cos());
        let c2 = (p3.0 + k * rx * t1.sin(), p3.1 - k * ry * t1.cos());
        ops.push(Op::C(c1, c2, p3));
    }
    ops
}

/// Consecutive subpaths: `counts[i]` points each, taken in order from `pts`. Counts that run past the
/// point list are cut short rather than failing the whole record.
pub(crate) fn subpaths(pts: &[P], counts: &[usize], close: bool) -> Vec<Op> {
    let mut ops = Vec::with_capacity(pts.len() + counts.len());
    let mut at = 0usize;
    for &c in counts {
        let end = at.saturating_add(c).min(pts.len());
        let Some((first, rest)) = pts.get(at..end).and_then(|s| s.split_first()) else {
            at = end;
            continue;
        };
        ops.push(Op::M(*first));
        ops.extend(rest.iter().map(|p| Op::L(*p)));
        if close {
            ops.push(Op::Z);
        }
        at = end;
    }
    ops
}
