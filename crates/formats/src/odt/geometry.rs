//! A custom shape's own geometry (ODF 1.3 §10.6.2 `draw:enhanced-geometry`): its path
//! (§19.145 `draw:enhanced-path`) in its view box, the path's values worked out from its
//! equations (§10.6.3 `draw:equation`, §19.170 `draw:formula`) and adjustment values
//! (`draw:modifiers`). Shapes whose type WordCraft has no kind for are drawn from it, as
//! freeforms.
//!
//! Arcs and ellipses are followed in short straight steps. Everything is bounded: the path's
//! length, the equations' count and nesting, the number of points.

use std::collections::HashMap;

use super::xml::{Attrs, get};
use wordcraft_doc::freeform::Freeform;
use wordcraft_doc::para::ShapeKind;

/// Most equations read, most levels one may refer down through, most path tokens and points.
const MAX_EQUATIONS: usize = 512;
const MAX_DEPTH: usize = 64;
const MAX_TOKENS: usize = 20_000;
const MAX_POINTS: usize = 20_000;
/// Farthest a point goes from the view box's origin, either way.
const MAX_COORD: f64 = 1e7;
/// Straight steps an arc of a full turn is followed in.
const ARC_STEPS: f64 = 64.0;

/// Longest path or formula kept, in bytes.
const MAX_TEXT: usize = 64 * 1024;

/// A custom shape's geometry being read: its path, view box and adjustment values, then its
/// equations, from the `draw:equation` children.
pub(super) struct Pending {
    path: String,
    vb: Option<[f32; 4]>,
    modifiers: String,
    equations: Vec<(String, String)>,
}

/// `s` cut to at most `MAX_TEXT` bytes, at a character boundary.
fn capped(s: &str) -> String {
    let mut end = s.len().min(MAX_TEXT);
    while !s.is_char_boundary(end) {
        end -= 1;
    }
    s.get(..end).unwrap_or_default().to_owned()
}

impl Pending {
    /// The geometry of a `draw:enhanced-geometry` of type `t` with attributes `a`, when it is
    /// to be drawn from its own path: a type WordCraft has no kind for, with a path.
    pub(super) fn of(t: &str, a: &Attrs) -> Option<Box<Pending>> {
        let t = t.trim();
        let plain = matches!(t.strip_prefix("ooxml-").unwrap_or(t), "" | "rectangle" | "rect" | "flowchart-process" | "flowChartProcess");
        if super::shapes::kind_of(t) != ShapeKind::Rectangle || plain {
            return None;
        }
        let path = get(a, "draw:enhanced-path")?;
        Some(Box::new(Pending {
            path: capped(path),
            vb: super::path::view_box(get(a, "svg:viewBox")),
            modifiers: capped(get(a, "draw:modifiers").unwrap_or_default()),
            equations: Vec::new(),
        }))
    }

    /// Reads a `draw:equation` (§19.171 `draw:name`, §19.170 `draw:formula`).
    pub(super) fn equation(&mut self, a: &Attrs) {
        if self.equations.len() >= MAX_EQUATIONS {
            return;
        }
        if let (Some(n), Some(f)) = (get(a, "draw:name"), get(a, "draw:formula")) {
            self.equations.push((capped(n.trim()), capped(f)));
        }
    }

    /// The freeform the geometry draws.
    pub(super) fn freeform(&self) -> Option<Freeform> {
        freeform(&self.path, self.vb, &self.modifiers, &self.equations)
    }
}

/// What a geometry's values are worked out from.
struct Env<'a> {
    vb: [f64; 4],
    modifiers: Vec<f64>,
    equations: &'a [(String, String)],
    names: HashMap<&'a str, usize>,
    /// Each equation worked out so far: its value, or `None` when it can't be (or is being worked
    /// out: one referring to itself fails), so none is worked out twice.
    memo: std::cell::RefCell<HashMap<usize, Option<f64>>>,
}

impl Env<'_> {
    /// The value of a path or formula parameter: a number, `?name` (an equation), `$n` (an
    /// adjustment value) or a keyword (§19.145).
    fn param(&self, t: &str, depth: usize) -> Option<f64> {
        if let Some(n) = t.strip_prefix('?') {
            return self.equation(*self.names.get(n)?, depth);
        }
        if let Some(i) = t.strip_prefix('$') {
            return self.modifiers.get(i.parse::<usize>().ok()?).copied();
        }
        let [x, y, w, h] = self.vb;
        Some(match t {
            "left" => x,
            "top" => y,
            "right" => x + w,
            "bottom" => y + h,
            "width" | "logwidth" => w,
            "height" | "logheight" => h,
            "xstretch" | "ystretch" => 0.0,
            "hasstroke" | "hasfill" => 1.0,
            "pi" => std::f64::consts::PI,
            _ => t.parse::<f64>().ok().filter(|v| v.is_finite())?,
        })
    }

    fn equation(&self, i: usize, depth: usize) -> Option<f64> {
        if depth > MAX_DEPTH {
            return None;
        }
        if let Some(v) = self.memo.borrow().get(&i) {
            return *v;
        }
        let f = &self.equations.get(i)?.1;
        self.memo.borrow_mut().insert(i, None);
        let mut p = Formula { s: f.as_bytes(), at: 0, env: self, depth: depth + 1 };
        let v = p.expr().map(|v| if v.is_finite() { v } else { 0.0 });
        self.memo.borrow_mut().insert(i, v);
        v
    }
}

/// A `draw:formula` being read (recursive descent: sums of products of factors).
struct Formula<'a, 'b> {
    s: &'a [u8],
    at: usize,
    env: &'b Env<'b>,
    depth: usize,
}

impl Formula<'_, '_> {
    fn skip(&mut self) {
        while self.s.get(self.at).is_some_and(u8::is_ascii_whitespace) {
            self.at += 1;
        }
    }

    fn peek(&mut self) -> Option<u8> {
        self.skip();
        self.s.get(self.at).copied()
    }

    fn expr(&mut self) -> Option<f64> {
        if self.depth > MAX_DEPTH {
            return None;
        }
        let mut v = self.term()?;
        while let Some(c @ (b'+' | b'-')) = self.peek() {
            self.at += 1;
            let r = self.term()?;
            v = if c == b'+' { v + r } else { v - r };
        }
        Some(v)
    }

    fn term(&mut self) -> Option<f64> {
        let mut v = self.factor()?;
        while let Some(c @ (b'*' | b'/')) = self.peek() {
            self.at += 1;
            let r = self.factor()?;
            v = if c == b'*' {
                v * r
            } else if r != 0.0 {
                v / r
            } else {
                0.0
            };
        }
        Some(v)
    }

    fn factor(&mut self) -> Option<f64> {
        self.depth += 1;
        if self.depth > MAX_DEPTH * 4 {
            return None;
        }
        let v = match self.peek()? {
            b'-' => {
                self.at += 1;
                -self.factor()?
            }
            b'+' => {
                self.at += 1;
                self.factor()?
            }
            b'(' => {
                self.at += 1;
                let v = self.expr()?;
                if self.peek() == Some(b')') {
                    self.at += 1;
                }
                v
            }
            _ => {
                let start = self.at;
                while self.s.get(self.at).is_some_and(|c| c.is_ascii_alphanumeric() || matches!(c, b'?' | b'$' | b'.' | b'_')) {
                    self.at += 1;
                }
                let word = std::str::from_utf8(self.s.get(start..self.at)?).ok()?;
                if word.is_empty() {
                    return None;
                }
                if self.peek() == Some(b'(') {
                    self.at += 1;
                    let mut args = Vec::new();
                    if self.peek() != Some(b')') {
                        loop {
                            args.push(self.expr()?);
                            match self.peek() {
                                Some(b',') => self.at += 1,
                                _ => break,
                            }
                            if args.len() > 8 {
                                return None;
                            }
                        }
                    }
                    if self.peek() == Some(b')') {
                        self.at += 1;
                    }
                    function(word, &args)?
                } else {
                    self.env.param(word, self.depth)?
                }
            }
        };
        self.depth -= 1;
        Some(v)
    }
}

/// A formula function (§19.170) of `args`.
fn function(name: &str, a: &[f64]) -> Option<f64> {
    let x = |i: usize| a.get(i).copied();
    Some(match name {
        "abs" => x(0)?.abs(),
        "sqrt" => x(0)?.max(0.0).sqrt(),
        "sin" => x(0)?.sin(),
        "cos" => x(0)?.cos(),
        "tan" => x(0)?.tan(),
        "atan" => x(0)?.atan(),
        "atan2" => x(0)?.atan2(x(1)?),
        "min" => x(0)?.min(x(1)?),
        "max" => x(0)?.max(x(1)?),
        "if" => {
            if x(0)? > 0.0 {
                x(1)?
            } else {
                x(2)?
            }
        }
        _ => return None,
    })
}

/// A path being followed: its SVG path data, the pen's place, and how many points so far.
#[derive(Default)]
struct Pen {
    d: String,
    at: (f64, f64),
    points: usize,
}

impl Pen {
    fn cmd(&mut self, c: char, pts: &[(f64, f64)]) {
        if self.points >= MAX_POINTS {
            return;
        }
        self.d.push(c);
        // Far outside any view box is as far as it goes: points stay finite as `f32`.
        let at = |v: f64| if v.is_finite() { v.clamp(-MAX_COORD, MAX_COORD) } else { 0.0 };
        let pts: Vec<(f64, f64)> = pts.iter().map(|(x, y)| (at(*x), at(*y))).collect();
        let pts = pts.as_slice();
        for (x, y) in pts {
            self.d.push_str(&format!(" {x:.2} {y:.2}"));
        }
        self.d.push(' ');
        self.points += pts.len().max(1);
        if let Some(p) = pts.last() {
            self.at = *p;
        }
    }

    /// An arc of the ellipse centred at `c` with radii `r`, from angle `a0` to `a1` (radians,
    /// the y axis down): a line (or a move, `jump`) to its start, then steps along it.
    fn arc(&mut self, c: (f64, f64), r: (f64, f64), a0: f64, a1: f64, jump: bool) {
        let point = |a: f64| (c.0 + r.0 * a.cos(), c.1 + r.1 * a.sin());
        let start = point(a0);
        // A line to the start, unless the pen is there already.
        if jump || self.points == 0 || (start.0 - self.at.0).hypot(start.1 - self.at.1) > 1e-6 {
            self.cmd(if jump || self.points == 0 { 'M' } else { 'L' }, &[start]);
        }
        let sweep = a1 - a0;
        let steps = ((sweep.abs() / std::f64::consts::TAU) * ARC_STEPS).ceil().clamp(1.0, ARC_STEPS) as usize;
        for k in 1..=steps {
            self.cmd('L', &[point(a0 + sweep * k as f64 / steps as f64)]);
        }
    }
}

/// The angle of `p` on the ellipse centred at `c` with radii `r` (radians).
fn angle_on(c: (f64, f64), r: (f64, f64), p: (f64, f64)) -> f64 {
    let (dx, dy) = ((p.0 - c.0) / r.0.max(1e-9), (p.1 - c.1) / r.1.max(1e-9));
    dy.atan2(dx)
}

/// The freeform an enhanced geometry draws: its `path` with adjustment values `modifiers` and
/// `equations` (name, formula), in view box `vb` (§19.570; the default 21600 square). `None`
/// when the path draws nothing.
pub(super) fn freeform(path: &str, vb: Option<[f32; 4]>, modifiers: &str, equations: &[(String, String)]) -> Option<Freeform> {
    let vb = vb.map_or([0.0, 0.0, 21600.0, 21600.0], |v| v.map(f64::from));
    if !(vb[2] > 0.0 && vb[3] > 0.0) {
        return None;
    }
    let equations = equations.get(..equations.len().min(MAX_EQUATIONS)).unwrap_or_default();
    let names = equations.iter().enumerate().map(|(i, (n, _))| (n.as_str(), i)).collect();
    let modifiers = modifiers.split_whitespace().take(64).map(|m| m.parse::<f64>().ok().filter(|v| v.is_finite()).unwrap_or(0.0)).collect();
    let env = Env { vb, modifiers, equations, names, memo: Default::default() };
    // The path's tokens: command letters, and parameters (numbers, ?names, $n, keywords).
    let tokens: Vec<&str> = path.split(|c: char| c.is_whitespace() || c == ',').filter(|t| !t.is_empty()).take(MAX_TOKENS).collect();
    let mut pen = Pen::default();
    let mut i = 0;
    let mut cmd = 'M';
    let is_cmd = |t: &str| t.len() == 1 && t.chars().all(|c| "MLCQZNFSTUABWVXY".contains(c));
    while i < tokens.len() {
        let t = tokens.get(i).copied().unwrap_or_default();
        if is_cmd(t) {
            cmd = t.chars().next().unwrap_or('M');
            i += 1;
            match cmd {
                'Z' => pen.cmd('Z', &[]),
                'N' | 'F' | 'S' => {}
                _ => continue,
            }
            continue;
        }
        // The command's parameters, as many as it takes each time.
        let n = match cmd {
            'M' | 'L' | 'X' | 'Y' => 2,
            'Q' => 4,
            'C' | 'T' | 'U' => 6,
            'A' | 'B' | 'W' | 'V' => 8,
            _ => {
                i += 1;
                continue;
            }
        };
        let Some(args) = tokens.get(i..i + n) else { break };
        i += n;
        let v: Option<Vec<f64>> = args.iter().map(|a| env.param(a, 0)).collect();
        let Some(v) = v else { continue };
        let p = |k: usize| (v.get(k).copied().unwrap_or(0.0), v.get(k + 1).copied().unwrap_or(0.0));
        match cmd {
            'M' => {
                pen.cmd('M', &[p(0)]);
                cmd = 'L';
            }
            'L' => pen.cmd('L', &[p(0)]),
            'Q' => pen.cmd('Q', &[p(0), p(2)]),
            'C' => pen.cmd('C', &[p(0), p(2), p(4)]),
            // An ellipse from angle to angle, in degrees (§19.145 T, U: U starts a new path).
            'T' | 'U' => {
                let (c, r) = (p(0), p(2));
                let (a0, a1) = (p(4).0.to_radians(), p(4).1.to_radians());
                let a1 = if a1 <= a0 { a1 + std::f64::consts::TAU } else { a1 };
                pen.arc(c, r, a0, a1, cmd == 'U');
            }
            // Arcs of the ellipse in a box, from where a ray to one point meets it to where a
            // ray to another does: anticlockwise (A, B) or clockwise (W, V); B and V start a new
            // path.
            'A' | 'B' | 'W' | 'V' => {
                let ((x1, y1), (x2, y2)) = (p(0), p(2));
                let c = ((x1 + x2) / 2.0, (y1 + y2) / 2.0);
                let r = (((x2 - x1) / 2.0).abs(), ((y2 - y1) / 2.0).abs());
                let (a0, a1) = (angle_on(c, r, p(4)), angle_on(c, r, p(6)));
                let clockwise = matches!(cmd, 'W' | 'V');
                // The y axis points down: clockwise on the page is increasing angles.
                let a1 = if clockwise {
                    if a1 <= a0 { a1 + std::f64::consts::TAU } else { a1 }
                } else if a1 >= a0 {
                    a1 - std::f64::consts::TAU
                } else {
                    a1
                };
                pen.arc(c, r, a0, a1, matches!(cmd, 'B' | 'V'));
            }
            // A quarter ellipse to a point, setting out along the x axis (X) or the y axis (Y),
            // the next one the other way.
            'X' | 'Y' => {
                let (from, to) = (pen.at, p(0));
                let (c, r, a0, a1) = if cmd == 'X' {
                    let c = (from.0, to.1);
                    let r = ((to.0 - from.0).abs(), (to.1 - from.1).abs());
                    (c, r, angle_on(c, r, from), angle_on(c, r, to))
                } else {
                    let c = (to.0, from.1);
                    let r = ((to.0 - from.0).abs(), (to.1 - from.1).abs());
                    (c, r, angle_on(c, r, from), angle_on(c, r, to))
                };
                // The short way round: a quarter.
                let mut a1 = a1;
                while a1 - a0 > std::f64::consts::PI {
                    a1 -= std::f64::consts::TAU;
                }
                while a0 - a1 > std::f64::consts::PI {
                    a1 += std::f64::consts::TAU;
                }
                pen.arc(c, r, a0, a1, false);
                cmd = if cmd == 'X' { 'Y' } else { 'X' };
            }
            _ => {}
        }
    }
    if pen.points < 2 {
        return None;
    }
    super::path::path(&pen.d, Some(vb.map(|v| v as f32)))
}
