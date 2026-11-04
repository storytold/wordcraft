//! Geometry of polygons, polylines, paths and regular polygons (ODF 1.3 §10.3.4–10.3.7, §19.1xx
//! `draw:points`, `svg:d`, `svg:viewBox`): their points, in their view box's coordinates, into
//! a WordCraft freeform (straight segments: curves and arcs are followed by short steps).

use wordcraft_doc::freeform::{FreePath, Freeform, MAX_PATHS, MAX_POINTS};

/// Steps a curve or arc is followed by.
const CURVE_STEPS: usize = 16;
/// Most numbers read from one `svg:d` or `draw:points` (hostile files).
const MAX_NUMBERS: usize = 200_000;
/// Largest coordinate kept.
const MAX_COORD: f64 = 1.0e6;

/// A view box (`svg:viewBox`, §19.570: min x, min y, width, height).
pub(super) fn view_box(v: Option<&str>) -> Option<[f32; 4]> {
    let n: Vec<f32> = numbers(v?).take(4).collect();
    let [x, y, w, h] = n.try_into().ok()?;
    Some([x, y, w, h])
}

/// The numbers in `s`, separated by spaces or commas (an SVG number list), at most
/// [`MAX_NUMBERS`]; what doesn't read as a number ends it.
fn numbers(s: &str) -> impl Iterator<Item = f32> + '_ {
    s.split(|c: char| c.is_whitespace() || c == ',')
        .filter(|t| !t.is_empty())
        .take(MAX_NUMBERS)
        .map_while(|t| t.parse::<f32>().ok())
        .map(|v| if v.is_finite() { v.clamp(-MAX_COORD as f32, MAX_COORD as f32) } else { 0.0 })
}

/// A freeform from `paths` (view box coordinates) in view box `vb` (none, or an empty one: the
/// box around the points): points from its corner.
fn freeform(paths: Vec<FreePath>, vb: Option<[f32; 4]>) -> Option<Freeform> {
    let [x0, y0, w, h] = match vb {
        Some(vb @ [_, _, w, h]) if w > 0.0 && h > 0.0 => vb,
        _ => {
            let all = || paths.iter().flat_map(|p| p.pts.iter());
            let (x0, y0) = all().fold((f32::MAX, f32::MAX), |(x, y), p| (x.min(p[0]), y.min(p[1])));
            let (x1, y1) = all().fold((f32::MIN, f32::MIN), |(x, y), p| (x.max(p[0]), y.max(p[1])));
            if x0 > x1 { [0.0, 0.0, 0.0, 0.0] } else { [x0, y0, x1 - x0, y1 - y0] }
        }
    };
    let paths: Vec<FreePath> = paths
        .into_iter()
        .take(MAX_PATHS)
        .map(|p| FreePath { pts: p.pts.into_iter().take(MAX_POINTS).map(|[x, y]| [x - x0, y - y0]).collect(), closed: p.closed })
        .filter(|p| !p.pts.is_empty())
        .collect();
    (!paths.is_empty()).then(|| Freeform { w: w.max(0.0), h: h.max(0.0), paths, ..Default::default() }.sanitized())
}

/// A polygon's or polyline's points (`draw:points`: "x,y x,y …"), closed or not.
pub(super) fn points(v: &str, closed: bool, vb: Option<[f32; 4]>) -> Option<Freeform> {
    let n: Vec<f32> = numbers(v).collect();
    let pts = n.as_chunks::<2>().0.iter().map(|c| [c[0], c[1]]).take(MAX_POINTS).collect();
    freeform(vec![FreePath { pts, closed }], vb)
}

/// A regular polygon (§10.3.6) of `corners` corners in a `w` × `h` box, its first corner at the
/// top middle; concave (a star), its inner corners `sharpness` (0–1) of the way in.
pub(super) fn regular(corners: u32, concave: bool, sharpness: f32, w: f32, h: f32) -> Option<Freeform> {
    let n = corners.clamp(3, 1000) as usize;
    let (rx, ry) = (f64::from(w) / 2.0, f64::from(h) / 2.0);
    let inner = 1.0 - f64::from(if sharpness.is_finite() { sharpness.clamp(0.0, 1.0) } else { 0.0 });
    let step = std::f64::consts::TAU / n as f64;
    let mut pts = Vec::with_capacity(n * 2);
    for i in 0..n {
        let a = -std::f64::consts::FRAC_PI_2 + step * i as f64;
        pts.push([(rx + rx * a.cos()) as f32, (ry + ry * a.sin()) as f32]);
        if concave {
            let b = a + step / 2.0;
            pts.push([(rx + rx * inner * b.cos()) as f32, (ry + ry * inner * b.sin()) as f32]);
        }
    }
    freeform(vec![FreePath { pts, closed: true }], Some([0.0, 0.0, w, h]))
}

/// A path's SVG path data (`svg:d`, §19.523: moves, lines, cubic and quadratic curves and
/// elliptical arcs, absolute and relative), each subpath a freeform path.
pub(super) fn path(d: &str, vb: Option<[f32; 4]>) -> Option<Freeform> {
    let mut p = PathReader::default();
    let mut tokens = Tokens { s: d, at: 0, read: 0, peeked: None };
    let mut cmd = None;
    while let Some(t) = tokens.next() {
        let c = match t {
            Token::Cmd(c) => c,
            // More numbers repeat the command (a move's are lines).
            Token::Num(_) => match cmd {
                Some('M') => 'L',
                Some('m') => 'l',
                // A close takes no numbers: a number after one ends the data (it would be read
                // again for ever otherwise).
                Some('Z' | 'z') | None => break,
                Some(c) => c,
            },
        };
        if let Token::Num(_) = t {
            tokens.peeked = Some(t);
        }
        cmd = Some(c);
        if !p.command(c, &mut tokens) {
            break;
        }
    }
    p.end(false);
    freeform(p.paths, vb)
}

#[derive(Clone, Copy)]
enum Token {
    Cmd(char),
    Num(f64),
}

/// The commands and numbers of path data.
struct Tokens<'a> {
    s: &'a str,
    at: usize,
    read: usize,
    /// A number read as the start of a repeated command, to be read again as its first.
    peeked: Option<Token>,
}

impl Tokens<'_> {
    fn next(&mut self) -> Option<Token> {
        if let Some(t) = self.peeked.take() {
            return Some(t);
        }
        self.read += 1;
        if self.read > MAX_NUMBERS {
            return None;
        }
        let rest = self.s.get(self.at..)?;
        let skip = rest.len() - rest.trim_start_matches(|c: char| c.is_whitespace() || c == ',').len();
        self.at += skip;
        let rest = self.s.get(self.at..)?;
        let c = rest.chars().next()?;
        if c.is_ascii_alphabetic() && c != 'e' && c != 'E' {
            self.at += c.len_utf8();
            return Some(Token::Cmd(c));
        }
        // A number: sign, digits, a point, an exponent; a second point or sign starts the next.
        let b = rest.as_bytes();
        let mut end = 0;
        let (mut dot, mut exp) = (false, false);
        while let Some(&ch) = b.get(end) {
            let ok = match ch {
                b'0'..=b'9' => true,
                b'+' | b'-' => end == 0 || matches!(b.get(end - 1), Some(b'e' | b'E')),
                b'.' if !dot && !exp => {
                    dot = true;
                    true
                }
                b'e' | b'E' if !exp && end > 0 => {
                    exp = true;
                    true
                }
                _ => false,
            };
            if !ok {
                break;
            }
            end += 1;
        }
        if end == 0 {
            // Not a number: stop.
            self.at = self.s.len();
            return None;
        }
        let v = rest.get(..end).and_then(|n| n.parse::<f64>().ok()).filter(|v| v.is_finite()).unwrap_or(0.0);
        self.at += end;
        Some(Token::Num(v.clamp(-MAX_COORD, MAX_COORD)))
    }

    fn num(&mut self) -> Option<f64> {
        match self.next() {
            Some(Token::Num(v)) => Some(v),
            Some(Token::Cmd(_)) => {
                // A command where a number belongs: the data is malformed; stop.
                self.at = self.s.len();
                None
            }
            None => None,
        }
    }
}

#[derive(Default)]
struct PathReader {
    paths: Vec<FreePath>,
    cur: Vec<[f32; 2]>,
    at: (f64, f64),
    start: (f64, f64),
    /// The last curve's second control point, for a smooth curve's reflection.
    last_ctrl: Option<(f64, f64, bool)>,
}

impl PathReader {
    fn push(&mut self, p: (f64, f64)) {
        if self.cur.len() < MAX_POINTS {
            let c = |v: f64| v.clamp(-MAX_COORD, MAX_COORD) as f32;
            self.cur.push([c(p.0), c(p.1)]);
        }
        self.at = p;
    }

    /// A drawing command without a move before it starts its subpath where the pen is.
    fn begin(&mut self) {
        if self.cur.is_empty() {
            self.push(self.at);
        }
    }

    fn end(&mut self, closed: bool) {
        let pts = std::mem::take(&mut self.cur);
        if !pts.is_empty() && self.paths.len() < MAX_PATHS {
            self.paths.push(FreePath { pts, closed });
        }
    }

    /// Command `c` with its numbers from `t`; false when the data runs out or is malformed.
    fn command(&mut self, c: char, t: &mut Tokens<'_>) -> bool {
        let rel = c.is_ascii_lowercase();
        let (ox, oy) = if rel { self.at } else { (0.0, 0.0) };
        let pt = |t: &mut Tokens<'_>| Some((t.num()? + ox, t.num()? + oy));
        let mut quad = false;
        match c.to_ascii_uppercase() {
            'M' => {
                let Some(p) = pt(t) else { return false };
                self.end(false);
                self.start = p;
                self.push(p);
            }
            'L' => {
                let Some(p) = pt(t) else { return false };
                self.begin();
                self.push(p);
            }
            'H' => {
                let Some(x) = t.num() else { return false };
                let x = if rel { x + self.at.0 } else { x };
                self.begin();
                self.push((x, self.at.1));
            }
            'V' => {
                let Some(y) = t.num() else { return false };
                let y = if rel { y + self.at.1 } else { y };
                self.begin();
                self.push((self.at.0, y));
            }
            'C' | 'S' => {
                let c1 = if c.eq_ignore_ascii_case(&'S') { self.reflected(false) } else { pt(t) };
                let (Some(c1), Some(c2), Some(e)) = (c1, pt(t), pt(t)) else { return false };
                self.cubic(c1, c2, e);
                self.last_ctrl = Some((c2.0, c2.1, false));
                return true;
            }
            'Q' | 'T' => {
                let q = if c.eq_ignore_ascii_case(&'T') { self.reflected(true) } else { pt(t) };
                let (Some(q), Some(e)) = (q, pt(t)) else { return false };
                // A quadratic curve is the cubic with controls two thirds of the way to its own.
                let s = self.at;
                let c1 = (s.0 + 2.0 / 3.0 * (q.0 - s.0), s.1 + 2.0 / 3.0 * (q.1 - s.1));
                let c2 = (e.0 + 2.0 / 3.0 * (q.0 - e.0), e.1 + 2.0 / 3.0 * (q.1 - e.1));
                self.cubic(c1, c2, e);
                self.last_ctrl = Some((q.0, q.1, true));
                quad = true;
            }
            'A' => {
                let (Some(rx), Some(ry), Some(rot), Some(large), Some(sweep), Some(e)) = (t.num(), t.num(), t.num(), t.num(), t.num(), pt(t)) else {
                    return false;
                };
                self.arc((rx, ry), rot, large != 0.0, sweep != 0.0, e);
            }
            'Z' => {
                self.end(true);
                // A new subpath after it starts where this one did, unless it moves.
                self.at = self.start;
            }
            _ => return false,
        }
        if !quad {
            self.last_ctrl = None;
        }
        true
    }

    /// The first control point of a smooth curve: the last one's reflected (`quad`: of a
    /// quadratic curve), or the current point.
    fn reflected(&self, quad: bool) -> Option<(f64, f64)> {
        Some(match self.last_ctrl {
            Some((x, y, q)) if q == quad => (2.0 * self.at.0 - x, 2.0 * self.at.1 - y),
            _ => self.at,
        })
    }

    fn cubic(&mut self, c1: (f64, f64), c2: (f64, f64), e: (f64, f64)) {
        let s = self.at;
        self.begin();
        for i in 1..CURVE_STEPS {
            let t = i as f64 / CURVE_STEPS as f64;
            let u = 1.0 - t;
            let (a, b, c, d) = (u * u * u, 3.0 * u * u * t, 3.0 * u * t * t, t * t * t);
            self.push((a * s.0 + b * c1.0 + c * c2.0 + d * e.0, a * s.1 + b * c1.1 + c * c2.1 + d * e.1));
        }
        self.push(e);
    }

    /// An elliptical arc to `e` (SVG's endpoint form, turned into its centre form).
    fn arc(&mut self, r: (f64, f64), rot_deg: f64, large: bool, sweep: bool, e: (f64, f64)) {
        let s = self.at;
        self.begin();
        let (mut rx, mut ry) = (r.0.abs(), r.1.abs());
        if rx < 1e-9 || ry < 1e-9 || (s.0 - e.0).abs() + (s.1 - e.1).abs() < 1e-9 {
            self.push(e);
            return;
        }
        let (sin, cos) = (rot_deg % 360.0).to_radians().sin_cos();
        // The start relative to the chord's middle, in the ellipse's own axes.
        let (dx, dy) = ((s.0 - e.0) / 2.0, (s.1 - e.1) / 2.0);
        let (x1, y1) = (cos * dx + sin * dy, -sin * dx + cos * dy);
        // Radii too small to reach are scaled up just enough.
        let k = (x1 * x1) / (rx * rx) + (y1 * y1) / (ry * ry);
        if k > 1.0 {
            (rx, ry) = (rx * k.sqrt(), ry * k.sqrt());
        }
        let num = (rx * rx * ry * ry - rx * rx * y1 * y1 - ry * ry * x1 * x1).max(0.0);
        let den = rx * rx * y1 * y1 + ry * ry * x1 * x1;
        let mut f = if den > 0.0 { (num / den).sqrt() } else { 0.0 };
        if large == sweep {
            f = -f;
        }
        let (cx1, cy1) = (f * rx * y1 / ry, -f * ry * x1 / rx);
        let (cx, cy) = (cos * cx1 - sin * cy1 + (s.0 + e.0) / 2.0, sin * cx1 + cos * cy1 + (s.1 + e.1) / 2.0);
        let angle = |ux: f64, uy: f64| uy.atan2(ux);
        let a0 = angle((x1 - cx1) / rx, (y1 - cy1) / ry);
        let a1 = angle((-x1 - cx1) / rx, (-y1 - cy1) / ry);
        let mut da = a1 - a0;
        if sweep && da < 0.0 {
            da += std::f64::consts::TAU;
        } else if !sweep && da > 0.0 {
            da -= std::f64::consts::TAU;
        }
        if !da.is_finite() || !cx.is_finite() || !cy.is_finite() {
            self.push(e);
            return;
        }
        for i in 1..CURVE_STEPS {
            let a = a0 + da * i as f64 / CURVE_STEPS as f64;
            let (px, py) = (rx * a.cos(), ry * a.sin());
            self.push((cos * px - sin * py + cx, sin * px + cos * py + cy));
        }
        self.push(e);
    }
}

/// `freeform`'s paths as SVG path data in a view box `vw` × `vh` (whole numbers, as ODF's view
/// box is): each path moved to, lined through, and closed when it is.
pub(super) fn path_data(f: &Freeform, vw: u32, vh: u32) -> String {
    path_text(f, vw, vh, false)
}

/// Whether `freeform` has both closed and open paths: drawn filled and unfilled.
pub(super) fn is_mixed(f: &Freeform) -> bool {
    f.paths.iter().any(|p| p.closed) && f.paths.iter().any(|p| !p.closed)
}

/// `freeform`'s paths as a custom shape's path (§19.145 `draw:enhanced-path`) in a view box
/// `vw` × `vh`: each path ended (`N`), open ones unfilled (`F`).
pub(super) fn enhanced_path_data(f: &Freeform, vw: u32, vh: u32) -> String {
    path_text(f, vw, vh, true)
}

fn path_text(f: &Freeform, vw: u32, vh: u32, enhanced: bool) -> String {
    let sx = if f.w > 0.0 { vw as f32 / f.w } else { 1.0 };
    let sy = if f.h > 0.0 { vh as f32 / f.h } else { 1.0 };
    let n = |v: f32| {
        let v = if v.is_finite() { v.clamp(-MAX_COORD as f32, MAX_COORD as f32) } else { 0.0 };
        format!("{}", v.round() as i64)
    };
    let mut d = String::new();
    for p in f.paths.iter().take(MAX_PATHS) {
        for (i, [x, y]) in p.pts.iter().take(MAX_POINTS).enumerate() {
            d.push_str(if i == 0 { "M" } else { " L" });
            if enhanced {
                d.push(' ');
            }
            d.push_str(&format!("{} {}", n(x * sx), n(y * sy)));
        }
        if p.closed {
            d.push_str(" Z");
        }
        if enhanced {
            d.push_str(if p.closed { " N" } else { " F N" });
        }
        d.push(' ');
    }
    d.trim_end().to_string()
}

#[cfg(test)]
mod tests {
    /// A number after a close ends the path data; it used to be read again for ever (a hang on a
    /// hostile or malformed `svg:d`).
    #[test]
    fn a_number_after_a_close_ends_the_path() {
        let f = super::path("M 0 0 L 10 0 L 10 10 Z 5 5", Some([0.0, 0.0, 10.0, 10.0]));
        let paths = f.map(|f| f.paths).unwrap_or_default();
        assert_eq!(paths.len(), 1, "{paths:?}");
        assert!(paths[0].closed);
        assert_eq!(paths[0].pts.len(), 3);
        // Lower case too, and a close ending the data as before.
        assert!(super::path("m 0 0 l 10 0 l 0 10 z 1", None).is_some());
        assert!(super::path("M 0 0 L 10 0 L 10 10 Z", None).is_some());
    }
}
