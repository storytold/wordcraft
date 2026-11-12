//! Draw › Ink to Shape: a small geometric recogniser that tells whether a hand-drawn stroke is a
//! straight line, a rectangle (or square), an ellipse (or circle), a triangle or another polygon
//! of up to eight corners. Our own fit: an open stroke is a line when it stays close to the chord
//! between its ends; a closed one is an ellipse when its points sit on the ellipse inscribed in
//! its bounds (low RMS of the normalised radius), else a polygon from its corners (the closed
//! path simplified with Ramer–Douglas–Peucker, near-straight and crowded corners merged) when
//! that polygon stays close to every point. An arrow is a line-like shaft plus a head drawn in the
//! same stroke (shaft first or head first): two barbs, or a small closed triangle round the tip.
//! [`pair_arrow`] makes one arrow of two strokes: a line, and a head (an open V or a small closed
//! triangle) whose apex sits at one of its ends. Anything else (scribbles, curves, letters) stays ink.

use wordcraft_doc::freeform::MAX_POINTS;

/// A shape recognised in a stroke, in the stroke's own coordinates.
#[derive(Clone, Debug, PartialEq)]
pub enum InkShape {
    /// A straight line from `a` to `b`.
    Line { a: [f32; 2], b: [f32; 2] },
    /// A straight arrow from the tail `a` to the tip `b`; the heads are the `head_at_*` flags
    /// (recognised strokes have one, at `b`).
    Arrow { a: [f32; 2], b: [f32; 2], head_at_end: bool, head_at_start: bool },
    /// An upright rectangle: top-left and size (equal sides when drawn nearly square).
    Rectangle { x: f32, y: f32, w: f32, h: f32 },
    /// An upright ellipse in this box (a circle when drawn nearly round).
    Ellipse { x: f32, y: f32, w: f32, h: f32 },
    /// A closed polygon through its 3–8 corners (a triangle has three).
    Polygon(Vec<[f32; 2]>),
}

impl InkShape {
    /// What it is, for messages and command results.
    pub fn name(&self) -> &'static str {
        match self {
            InkShape::Line { .. } => "line",
            InkShape::Arrow { .. } => "arrow",
            InkShape::Rectangle { w, h, .. } if w == h => "square",
            InkShape::Rectangle { .. } => "rectangle",
            InkShape::Ellipse { w, h, .. } if w == h => "circle",
            InkShape::Ellipse { .. } => "ellipse",
            InkShape::Polygon(c) if c.len() == 3 => "triangle",
            InkShape::Polygon(_) => "polygon",
        }
    }

    /// Its bounds: left, top, width, height.
    pub fn bounds(&self) -> [f32; 4] {
        match self {
            InkShape::Rectangle { x, y, w, h } | InkShape::Ellipse { x, y, w, h } => [*x, *y, *w, *h],
            InkShape::Line { a, b } | InkShape::Arrow { a, b, .. } => bbox(&[*a, *b]),
            InkShape::Polygon(c) => bbox(c),
        }
    }
}

/// Smallest stroke (its longer side, points) that is turned into a shape: a tap or a tick stays ink.
const MIN_SIZE: f32 = 6.0;
/// Most corners a polygon may have.
pub const MAX_CORNERS: usize = 8;

#[cfg(test)]
thread_local! {
    /// How many times [`recognise`] ran on this thread (tests check the work stays bounded).
    pub static RECOGNISED: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// The shape `pts` draws, if it is one.
pub fn recognise(pts: &[[f32; 2]]) -> Option<InkShape> {
    #[cfg(test)]
    RECOGNISED.with(|c| c.set(c.get().saturating_add(1)));
    let p = clean(pts);
    let (first, last) = (*p.first()?, *p.last()?);
    if p.len() < 2 {
        return None;
    }
    let [x0, y0, w, h] = bbox(&p);
    let size = w.max(h);
    let diag = w.hypot(h);
    if size < MIN_SIZE {
        return None;
    }
    let length: f32 = p.windows(2).map(|s| s.first().zip(s.get(1)).map_or(0.0, |(a, b)| dist(*a, *b))).sum();
    let gap = dist(first, last);
    // A line: the stroke barely leaves the chord between its ends and doesn't double back.
    if gap > 0.85 * length && gap >= MIN_SIZE {
        let off = p.iter().map(|q| seg_distance(first, last, *q)).fold(0.0, f32::max);
        if off <= (0.06 * gap).max(1.5) {
            return Some(InkShape::Line { a: first, b: last });
        }
    }
    // An arrow: a shaft and a head (both barbs, or a closed triangle), drawn in one stroke
    // (either end first).
    let rev: Vec<[f32; 2]> = p.iter().rev().copied().collect();
    if let Some(arrow) = arrow(&p).or_else(|| arrow(&rev)).or_else(|| closed_arrow(&p)).or_else(|| closed_arrow(&rev)) {
        return Some(arrow);
    }
    // Other shapes are closed: the stroke ends near where it began and goes around.
    if gap > 0.2 * size || length < 2.0 * size {
        return None;
    }
    // Ellipse: the points sit on the ellipse inscribed in their bounds.
    let ellipse_rms = ellipse_rms(&p, x0, y0, w, h);
    if w > 0.2 * size && h > 0.2 * size && ellipse_rms < 0.06 {
        let (x, y, w, h) = snap_square(x0, y0, w, h);
        return Some(InkShape::Ellipse { x, y, w, h });
    }
    // Polygon: the corners of the closed path, if they describe it.
    let corners = corners(&p, diag);
    let fits = corners.len() >= 3 && p.iter().all(|q| polygon_distance(&corners, *q) <= 0.08 * diag);
    if fits && corners.len() == 4 && upright(&corners) {
        let [x, y, w, h] = bbox(&corners);
        let (x, y, w, h) = snap_square(x, y, w, h);
        return Some(InkShape::Rectangle { x, y, w, h });
    }
    if fits && corners.len() <= MAX_CORNERS {
        return Some(InkShape::Polygon(corners));
    }
    // A wobbly round stroke is still an ellipse.
    (w > 0.2 * size && h > 0.2 * size && ellipse_rms < 0.1).then(|| {
        let (x, y, w, h) = snap_square(x0, y0, w, h);
        InkShape::Ellipse { x, y, w, h }
    })
}

/// Barb length limits, as a fraction of the shaft, and the angle each barb makes with the shaft
/// (degrees).
const BARB_LEN: (f32, f32) = (0.05, 0.5);
const BARB_ANGLE: (f32, f32) = (15.0, 75.0);

/// An arrow drawn shaft first: the shaft runs from the start to the tip (the point farthest from
/// the start) and stays near its chord like a line does; then the pen draws one barb, comes back
/// to the tip and draws the other. The barbs must leave the tip on opposite sides of the shaft at
/// similar angles and lengths. (A stroke drawn head first is tried reversed by the caller.)
fn arrow(p: &[[f32; 2]]) -> Option<InkShape> {
    let tail = *p.first()?;
    let (ti, tip, shaft) = shaft_tip(p)?;
    // The head: the stroke after the tip leaves it twice, once for each barb.
    let head = p.get(ti..)?;
    let reach = head.iter().map(|q| dist(tip, *q)).fold(0.0, f32::max);
    let (mut barbs, mut out, mut gap): (Vec<(f32, [f32; 2])>, bool, f32) = (Vec::new(), false, f32::INFINITY);
    for q in head {
        let d = dist(tip, *q);
        if d > 0.25 * reach {
            if !out {
                if barbs.len() == 2 {
                    return None;
                }
                barbs.push((d, *q));
                out = true;
            } else if let Some(b) = barbs.last_mut().filter(|b| d > b.0) {
                *b = (d, *q);
            }
        } else {
            out = false;
            if barbs.len() == 1 {
                gap = gap.min(d);
            }
        }
    }
    let &[(l1, b1), (l2, b2)] = barbs.as_slice() else { return None };
    // The pen returns to the tip between the barbs.
    if gap > 0.35 * l1.min(l2) || shaft <= 0.0 {
        return None;
    }
    head_fits(tail, tip, tip, b1, b2).then_some(InkShape::Arrow { a: tail, b: tip, head_at_end: true, head_at_start: false })
}

/// An arrow with a closed triangle head, drawn shaft first: the shaft runs from the start to the
/// tip like [`arrow`]'s, then the pen goes round a small triangle (tip, one barb end, the other)
/// and ends back at the tip. (Drawn head first, the caller tries it reversed.)
fn closed_arrow(p: &[[f32; 2]]) -> Option<InkShape> {
    let tail = *p.first()?;
    let (ti, tip, _) = shaft_tip(p)?;
    let head = p.get(ti..)?;
    // The corners: the point farthest from the tip, then the one farthest from that side.
    let b1 = head.iter().copied().fold(tip, |m, q| if dist(tip, q) > dist(tip, m) { q } else { m });
    let b2 = head.iter().copied().fold(tip, |m, q| if seg_distance(tip, b1, q) > seg_distance(tip, b1, m) { q } else { m });
    let (l1, l2) = (dist(tip, b1), dist(tip, b2));
    if l1.min(l2) <= 0.0 || dist(*head.last()?, tip) > 0.35 * l1.min(l2) {
        return None;
    }
    // Every point of the head is on the triangle.
    let tri = [tip, b1, b2];
    if head.iter().any(|q| polygon_distance(&tri, *q) > (0.15 * l1).max(1.5)) {
        return None;
    }
    head_fits(tail, tip, tip, b1, b2).then_some(InkShape::Arrow { a: tail, b: tip, head_at_end: true, head_at_start: false })
}

/// Whether a head whose barbs run from `apex` to `b1` and `b2` fits a shaft from `tail` to `tip`:
/// barbs 5–50 % of the shaft, each 15–75° off the way back along it, on opposite sides of it,
/// and of similar length and angle.
fn head_fits(tail: [f32; 2], tip: [f32; 2], apex: [f32; 2], b1: [f32; 2], b2: [f32; 2]) -> bool {
    let shaft = dist(tail, tip);
    let (l1, l2) = (dist(apex, b1), dist(apex, b2));
    if !(shaft > 0.0 && l1 > 0.0 && l2 > 0.0) {
        return false;
    }
    // Each barb as a vector from the apex, against the direction back along the shaft.
    let back = [tail[0] - tip[0], tail[1] - tip[1]];
    let angle_side = |e: [f32; 2], len: f32| {
        let u = [e[0] - apex[0], e[1] - apex[1]];
        let angle = ((back[0] * u[0] + back[1] * u[1]) / (shaft * len)).clamp(-1.0, 1.0).acos().to_degrees();
        (angle, back[0] * u[1] - back[1] * u[0])
    };
    let ((a1, s1), (a2, s2)) = (angle_side(b1, l1), angle_side(b2, l2));
    let ok = |l: f32, a: f32| (BARB_LEN.0 * shaft..=BARB_LEN.1 * shaft).contains(&l) && (BARB_ANGLE.0..=BARB_ANGLE.1).contains(&a);
    let similar = l1.max(l2) <= 2.5 * l1.min(l2) && (a1 - a2).abs() <= 35.0;
    ok(l1, a1) && ok(l2, a2) && s1 * s2 < 0.0 && similar
}

/// How far a two-stroke arrow's head apex may sit from the line's end, as a fraction of the line.
const PAIR_REACH: f32 = 0.12;

/// What a stroke can be in a two-stroke arrow, worked out once (one recognition): the ends of the
/// straight line it is, if it is one, and the ways it can be an arrowhead on its own.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ArrowParts {
    pub line: Option<([f32; 2], [f32; 2])>,
    pub heads: Vec<[[f32; 2]; 3]>,
}

/// `pts` as a possible half of a two-stroke arrow (see [`pair_parts`]).
pub fn arrow_parts(pts: &[[f32; 2]]) -> ArrowParts {
    let p = clean(pts);
    let found = recognise(&p);
    let line = match found {
        Some(InkShape::Line { a, b }) => Some((a, b)),
        _ => None,
    };
    ArrowParts { line, heads: heads(&p, found.as_ref()) }
}

/// One arrow from two strokes: `line`, a straight stroke, and `head`, an open V (one barb end,
/// the apex, the other end) or a small closed triangle, whose apex is at one end of the line and
/// whose barbs point back along it (the single-stroke head limits). The arrow runs from the
/// line's other end to that end; `None` when they don't make one.
pub fn pair_arrow(line: &[[f32; 2]], head: &[[f32; 2]]) -> Option<InkShape> {
    pair_parts(&arrow_parts(line), &arrow_parts(head))
}

/// [`pair_arrow`] on strokes already recognised by [`arrow_parts`]: cheap geometry only.
pub fn pair_parts(line: &ArrowParts, head: &ArrowParts) -> Option<InkShape> {
    let (a, b) = line.line?;
    let mut best: Option<(f32, InkShape)> = None;
    for &[apex, b1, b2] in &head.heads {
        for (tail, tip) in [(a, b), (b, a)] {
            let off = dist(apex, tip);
            let near = off <= (PAIR_REACH * dist(tail, tip)).max(3.0);
            if near && best.as_ref().is_none_or(|(d, _)| off < *d) && head_fits(tail, tip, apex, b1, b2) {
                best = Some((off, InkShape::Arrow { a: tail, b: tip, head_at_end: true, head_at_start: false }));
            }
        }
    }
    best.map(|(_, arrow)| arrow)
}

/// The ways the cleaned points `p` (which recognise as `found`) can be an arrowhead on their own,
/// as `[apex, barb end, barb end]`: each corner of a closed triangle, or the corner of an open V
/// whose two arms are straight.
fn heads(p: &[[f32; 2]], found: Option<&InkShape>) -> Vec<[[f32; 2]; 3]> {
    if let Some(InkShape::Polygon(c)) = found
        && let &[x, y, z] = c.as_slice()
    {
        return vec![[x, y, z], [y, z, x], [z, x, y]];
    }
    let (Some(&f), Some(&l)) = (p.first(), p.last()) else { return Vec::new() };
    // The apex: the point farthest from the chord between the ends.
    let Some((ai, apex)) = p.iter().copied().enumerate().fold(None, |m: Option<(usize, [f32; 2])>, (i, q)| match m {
        Some((_, b)) if seg_distance(f, l, b) >= seg_distance(f, l, q) => m,
        _ => Some((i, q)),
    }) else {
        return Vec::new();
    };
    let straight = |a: [f32; 2], b: [f32; 2], arm: Option<&[[f32; 2]]>| {
        let len = dist(a, b);
        len >= 1.0 && arm.is_some_and(|arm| arm.iter().all(|q| seg_distance(a, b, *q) <= (0.12 * len).max(1.5)))
    };
    if ai == 0 || ai + 1 >= p.len() || !straight(f, apex, p.get(..=ai)) || !straight(apex, l, p.get(ai..)) {
        return Vec::new();
    }
    vec![[apex, f, l]]
}

/// Finite, not absurdly far points only (at most [`MAX_POINTS`]), without repeats.
fn clean(pts: &[[f32; 2]]) -> Vec<[f32; 2]> {
    let mut p: Vec<[f32; 2]> = Vec::with_capacity(pts.len().min(MAX_POINTS));
    for q in pts.iter().take(MAX_POINTS).filter(|[x, y]| x.is_finite() && y.is_finite() && x.abs() < 1e6 && y.abs() < 1e6) {
        if p.last() != Some(q) {
            p.push(*q);
        }
    }
    p
}

/// A shaft drawn from the start of `p`: the index of its tip, the tip and the shaft's length;
/// `None` when it is too short or not straight.
fn shaft_tip(p: &[[f32; 2]]) -> Option<(usize, [f32; 2], f32)> {
    let tail = *p.first()?;
    // The tip: start at the first point about as far from the start as any (a return that
    // overshoots it by a point or two must not move it), then follow the stroke on while it still
    // moves away from the start (densely sampled strokes have shaft points inside that window).
    // The first barb heads back, so the walk stops at the real tip and never reaches an overshoot;
    // jitter of under half a point doesn't stop it early.
    let dmax = p.iter().map(|q| dist(tail, *q)).fold(0.0, f32::max);
    let mut ti = p.iter().position(|q| dist(tail, *q) >= dmax - (0.03 * dmax).max(2.0))?;
    let mut best = dist(tail, *p.get(ti)?);
    for (i, q) in p.iter().enumerate().skip(ti + 1) {
        let d = dist(tail, *q);
        if d >= best {
            (ti, best) = (i, d);
        } else if d < best - 0.5 {
            break;
        }
    }
    let tip = *p.get(ti)?;
    let shaft = dist(tail, tip);
    if shaft < MIN_SIZE || p.get(..=ti)?.iter().any(|q| seg_distance(tail, tip, *q) > (0.06 * shaft).max(1.5)) {
        return None;
    }
    Some((ti, tip, shaft))
}

/// Left, top, width and height of `pts` (zero for none).
fn bbox(pts: &[[f32; 2]]) -> [f32; 4] {
    let Some(f) = pts.first() else { return [0.0; 4] };
    let (mut x0, mut y0, mut x1, mut y1) = (f[0], f[1], f[0], f[1]);
    for [x, y] in pts {
        (x0, y0, x1, y1) = (x0.min(*x), y0.min(*y), x1.max(*x), y1.max(*y));
    }
    [x0, y0, x1 - x0, y1 - y0]
}

fn dist(a: [f32; 2], b: [f32; 2]) -> f32 {
    (b[0] - a[0]).hypot(b[1] - a[1])
}

/// Distance from `p` to the segment `a`–`b`.
fn seg_distance(a: [f32; 2], b: [f32; 2], p: [f32; 2]) -> f32 {
    let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
    let len2 = dx * dx + dy * dy;
    let t = if len2 > 0.0 { (((p[0] - a[0]) * dx + (p[1] - a[1]) * dy) / len2).clamp(0.0, 1.0) } else { 0.0 };
    dist([a[0] + t * dx, a[1] + t * dy], p)
}

/// Distance from `p` to the closed polygon through `c`.
fn polygon_distance(c: &[[f32; 2]], p: [f32; 2]) -> f32 {
    let n = c.len();
    (0..n).filter_map(|i| Some(seg_distance(*c.get(i)?, *c.get((i + 1) % n)?, p))).fold(f32::INFINITY, f32::min)
}

/// How far the points stray from the ellipse inscribed in the `w` × `h` box at (`x0`, `y0`): the
/// RMS of each point's normalised radius minus one (0 on the ellipse).
fn ellipse_rms(p: &[[f32; 2]], x0: f32, y0: f32, w: f32, h: f32) -> f32 {
    let (rx, ry) = (w / 2.0, h / 2.0);
    if rx <= 0.0 || ry <= 0.0 || p.is_empty() {
        return f32::INFINITY;
    }
    let (cx, cy) = (x0 + rx, y0 + ry);
    let sum: f32 = p.iter().map(|[x, y]| (((x - cx) / rx).hypot((y - cy) / ry) - 1.0).powi(2)).sum();
    (sum / p.len() as f32).sqrt()
}

/// A box drawn nearly square made square (same centre).
fn snap_square(x: f32, y: f32, w: f32, h: f32) -> (f32, f32, f32, f32) {
    if (w - h).abs() <= 0.12 * w.max(h) {
        let s = (w + h) / 2.0;
        (x + (w - s) / 2.0, y + (h - s) / 2.0, s, s)
    } else {
        (x, y, w, h)
    }
}

/// The corners of the closed path through `p`: simplified with Ramer–Douglas–Peucker (split
/// at the point farthest from the start), then corners that barely turn or crowd a neighbour
/// merged away.
fn corners(p: &[[f32; 2]], diag: f32) -> Vec<[f32; 2]> {
    let eps = 0.05 * diag;
    let Some(&start) = p.first() else { return Vec::new() };
    let far = p.iter().enumerate().fold((0, 0.0), |best, (i, q)| if dist(start, *q) > best.1 { (i, dist(start, *q)) } else { best }).0;
    let (a, b) = p.split_at(far.min(p.len()));
    let mut ring: Vec<[f32; 2]> = rdp(&[a, b.get(..1).unwrap_or(&[])].concat(), eps);
    let mut back = rdp(&[b, &[start]].concat(), eps);
    // The halves share the far point and the start.
    ring.pop();
    back.pop();
    ring.extend(back);
    // Merge until every corner turns sharply and none sits on top of the next.
    while ring.len() > 3 {
        let n = ring.len();
        let turn = |i: usize| -> f32 {
            let (Some(a), Some(b), Some(c)) = (ring.get((i + n - 1) % n), ring.get(i), ring.get((i + 1) % n)) else { return 0.0 };
            turning(*a, *b, *c)
        };
        let (i, t) = (0..n).map(|i| (i, turn(i))).fold((0, f32::INFINITY), |m, x| if x.1 < m.1 { x } else { m });
        let short = (0..n).find(|&j| ring.get(j).zip(ring.get((j + 1) % n)).is_some_and(|(a, b)| dist(*a, *b) < 0.1 * diag));
        if t < 30.0 {
            ring.remove(i);
        } else if let Some(j) = short {
            // Of the two crowded corners keep the sharper.
            let k = if turn(j) < turn((j + 1) % n) { j } else { (j + 1) % n };
            ring.remove(k);
        } else {
            break;
        }
    }
    ring
}

/// How far the path turns at `b` coming from `a` and going on to `c`, degrees (0 = straight on).
fn turning(a: [f32; 2], b: [f32; 2], c: [f32; 2]) -> f32 {
    let (u, v) = ([b[0] - a[0], b[1] - a[1]], [c[0] - b[0], c[1] - b[1]]);
    let (lu, lv) = (u[0].hypot(u[1]), v[0].hypot(v[1]));
    if lu <= 0.0 || lv <= 0.0 {
        return 0.0;
    }
    ((u[0] * v[0] + u[1] * v[1]) / (lu * lv)).clamp(-1.0, 1.0).acos().to_degrees()
}

/// Ramer–Douglas–Peucker simplification of an open path (iterative: no deep recursion on long
/// strokes); keeps both ends.
fn rdp(p: &[[f32; 2]], eps: f32) -> Vec<[f32; 2]> {
    let n = p.len();
    if n < 3 {
        return p.to_vec();
    }
    let mut keep = vec![false; n];
    if let Some(f) = keep.first_mut() {
        *f = true;
    }
    if let Some(l) = keep.last_mut() {
        *l = true;
    }
    let mut stack = vec![(0usize, n - 1)];
    while let Some((a, b)) = stack.pop() {
        let (Some(&pa), Some(&pb)) = (p.get(a), p.get(b)) else { continue };
        let far = (a + 1..b).filter_map(|i| Some((i, seg_distance(pa, pb, *p.get(i)?)))).fold((a, 0.0), |m, x| if x.1 > m.1 { x } else { m });
        if far.1 > eps
            && let Some(k) = keep.get_mut(far.0)
        {
            *k = true;
            stack.push((a, far.0));
            stack.push((far.0, b));
        }
    }
    p.iter().zip(keep).filter(|(_, k)| *k).map(|(q, _)| *q).collect()
}

/// A quadrilateral whose sides run (nearly) across and down the page, alternately.
fn upright(c: &[[f32; 2]]) -> bool {
    let n = c.len();
    let dirs: Vec<Option<bool>> = (0..n)
        .map(|i| {
            let (a, b) = (c.get(i)?, c.get((i + 1) % n)?);
            let deg = (b[1] - a[1]).atan2(b[0] - a[0]).to_degrees().rem_euclid(180.0);
            // true: across, false: down; None when it's slanted.
            if deg <= 15.0 || deg >= 165.0 {
                Some(true)
            } else if (75.0..=105.0).contains(&deg) {
                Some(false)
            } else {
                None
            }
        })
        .collect();
    dirs.iter().all(Option::is_some) && (0..n).all(|i| dirs.get(i) != dirs.get((i + 1) % n))
}

#[cfg(test)]
mod tests;
