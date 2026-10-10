//! WordCraft geometry and units.
//!
//! Layout works in **points** (1/72 inch, `f32`). File formats use other units: OOXML uses
//! twentieths of a point (twips) for most lengths, half-points for font sizes, eighths of a point
//! for borders and English Metric Units (EMU, 914 400 per inch) for drawings. The conversions live
//! here so every crate agrees on them.
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

use serde::{Deserialize, Serialize};

/// Points per inch.
pub const PT_PER_IN: f32 = 72.0;
/// Twips (1/20 pt) per point.
pub const TWIPS_PER_PT: f32 = 20.0;
/// English Metric Units per point.
pub const EMU_PER_PT: f32 = 12_700.0;
/// Points per centimetre.
pub const PT_PER_CM: f32 = 72.0 / 2.54;

/// Twips → points.
pub fn twips(v: i64) -> f32 {
    v as f32 / TWIPS_PER_PT
}
/// Points → twips (rounded).
pub fn to_twips(pt: f32) -> i64 {
    finite(pt * TWIPS_PER_PT).round() as i64
}
/// EMU → points.
pub fn emu(v: i64) -> f32 {
    v as f32 / EMU_PER_PT
}
/// Points → EMU (rounded).
pub fn to_emu(pt: f32) -> i64 {
    (finite(pt) as f64 * EMU_PER_PT as f64).round() as i64
}
/// Half-points (OOXML font sizes) → points.
pub fn half_points(v: i64) -> f32 {
    v as f32 / 2.0
}
/// Points → half-points (rounded).
pub fn to_half_points(pt: f32) -> i64 {
    (finite(pt) * 2.0).round() as i64
}

/// Replace NaN/inf with 0 so casts and arithmetic stay defined.
pub fn finite(v: f32) -> f32 {
    if v.is_finite() { v } else { 0.0 }
}

/// A measurement unit shown in the UI (Word's Options › Advanced › "Show measurements in units of").
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum Unit {
    #[default]
    Inches,
    Centimeters,
    Millimeters,
    Points,
    Picas,
}

impl Unit {
    pub fn suffix(self) -> &'static str {
        match self {
            Unit::Inches => "\"",
            Unit::Centimeters => " cm",
            Unit::Millimeters => " mm",
            Unit::Points => " pt",
            Unit::Picas => " pi",
        }
    }
    pub fn pt_per_unit(self) -> f32 {
        match self {
            Unit::Inches => PT_PER_IN,
            Unit::Centimeters => PT_PER_CM,
            Unit::Millimeters => PT_PER_CM / 10.0,
            Unit::Points => 1.0,
            Unit::Picas => 12.0,
        }
    }
    /// Format `pt` in this unit the way Word's fields show it (`1"`, `2.54 cm`, `12 pt`).
    pub fn format(self, pt: f32) -> String {
        let v = finite(pt) / self.pt_per_unit();
        let s = format!("{v:.2}");
        let s = s.trim_end_matches('0').trim_end_matches('.');
        let s = if s == "-0" { "0" } else { s };
        format!("{s}{}", self.suffix())
    }
}

/// Parse a measurement typed by a user: `1"`, `1 in`, `2.5cm`, `12pt`, `3 pi`, `10 mm`, `1.5 li`
/// (lines are 12 pt). A bare number is in `default`. Returns points.
pub fn parse_measure(s: &str, default: Unit) -> Option<f32> {
    let t = s.trim().to_ascii_lowercase();
    if t.is_empty() {
        return None;
    }
    let split = t.find(|c: char| !(c.is_ascii_digit() || c == '.' || c == '-' || c == '+' || c == ',')).unwrap_or(t.len());
    let (num, unit) = t.split_at(split);
    let v: f32 = num.replace(',', ".").parse().ok()?;
    if !v.is_finite() {
        return None;
    }
    let k = match unit.trim() {
        "" => default.pt_per_unit(),
        "\"" | "in" | "inch" | "inches" => PT_PER_IN,
        "cm" => PT_PER_CM,
        "mm" => PT_PER_CM / 10.0,
        "pt" | "pts" | "point" | "points" => 1.0,
        "pi" | "pc" | "pica" | "picas" => 12.0,
        "li" | "line" | "lines" => 12.0,
        "px" => 0.75,
        _ => return None,
    };
    Some(v * k)
}

/// A point in page space (points, y down).
#[derive(Clone, Copy, Debug, PartialEq, Default, Serialize, Deserialize)]
pub struct Point {
    pub x: f32,
    pub y: f32,
}

impl Point {
    pub const fn new(x: f32, y: f32) -> Self {
        Point { x, y }
    }
}

/// An axis-aligned rectangle (points, y down). `w`/`h` are never negative when built with [`Rect::new`].
#[derive(Clone, Copy, Debug, PartialEq, Default, Serialize, Deserialize)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

impl Rect {
    pub fn new(x: f32, y: f32, w: f32, h: f32) -> Self {
        Rect { x: finite(x), y: finite(y), w: finite(w).max(0.0), h: finite(h).max(0.0) }
    }
    pub fn right(&self) -> f32 {
        self.x + self.w
    }
    pub fn bottom(&self) -> f32 {
        self.y + self.h
    }
    pub fn contains(&self, p: Point) -> bool {
        p.x >= self.x && p.x <= self.right() && p.y >= self.y && p.y <= self.bottom()
    }
    pub fn intersects(&self, o: &Rect) -> bool {
        self.x < o.right() && o.x < self.right() && self.y < o.bottom() && o.y < self.bottom()
    }
    pub fn union(&self, o: &Rect) -> Rect {
        let x = self.x.min(o.x);
        let y = self.y.min(o.y);
        Rect::new(x, y, self.right().max(o.right()) - x, self.bottom().max(o.bottom()) - y)
    }
    pub fn inset(&self, l: f32, t: f32, r: f32, b: f32) -> Rect {
        Rect::new(self.x + l, self.y + t, self.w - l - r, self.h - t - b)
    }
    /// Grown by `d` on every side (shrunk for negative `d`, down to zero size).
    pub fn expand(&self, d: f32) -> Rect {
        self.inset(-d, -d, -d, -d)
    }
}

/// An angle in degrees brought into `0..360` (NaN and infinities become 0).
pub fn normalize_degrees(deg: f32) -> f32 {
    if !deg.is_finite() {
        return 0.0;
    }
    let d = deg.rem_euclid(360.0);
    // `rem_euclid` can round up to exactly 360 for tiny negative angles.
    if (0.0..360.0).contains(&d) { d } else { 0.0 }
}

/// How a drawing is turned about its centre: mirrored first (`flip_h` left to right, `flip_v` top
/// to bottom), then rotated `deg` degrees clockwise (page space, y down), as DrawingML's `a:xfrm`
/// `flipH`, `flipV` and `rot` say (ECMA-376 §20.1.7.6).
#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub struct Spin {
    pub deg: f32,
    pub flip_h: bool,
    pub flip_v: bool,
}

impl Spin {
    /// A spin with its angle normalised to `0..360`.
    pub fn new(deg: f32, flip_h: bool, flip_v: bool) -> Spin {
        Spin { deg: normalize_degrees(deg), flip_h, flip_v }
    }

    /// No rotation and no mirroring.
    pub fn is_identity(&self) -> bool {
        normalize_degrees(self.deg) == 0.0 && !self.flip_h && !self.flip_v
    }

    /// The linear part (a, b, c, d: x' = a·x + c·y, y' = b·x + d·y).
    fn linear(&self) -> [f32; 4] {
        let r = normalize_degrees(self.deg).to_radians();
        let (s, c) = r.sin_cos();
        let fx = if self.flip_h { -1.0 } else { 1.0 };
        let fy = if self.flip_v { -1.0 } else { 1.0 };
        [c * fx, s * fx, -s * fy, c * fy]
    }

    /// The affine map (a, b, c, d, e, f: x' = a·x + c·y + e, y' = b·x + d·y + f) turning the page
    /// about point (`cx`, `cy`).
    pub fn matrix(&self, cx: f32, cy: f32) -> [f32; 6] {
        let [a, b, c, d] = self.linear();
        let (cx, cy) = (finite(cx), finite(cy));
        [a, b, c, d, cx - (a * cx + c * cy), cy - (b * cx + d * cy)]
    }

    /// Where point (`x`, `y`) goes when turned about (`cx`, `cy`).
    pub fn apply(&self, cx: f32, cy: f32, x: f32, y: f32) -> (f32, f32) {
        let [a, b, c, d] = self.linear();
        let (u, v) = (x - cx, y - cy);
        (cx + a * u + c * v, cy + b * u + d * v)
    }

    /// The point that [`Spin::apply`] takes to (`x`, `y`).
    pub fn unapply(&self, cx: f32, cy: f32, x: f32, y: f32) -> (f32, f32) {
        // The linear part is orthogonal: its inverse is its transpose.
        let [a, b, c, d] = self.linear();
        let (u, v) = (x - cx, y - cy);
        (cx + a * u + b * v, cy + c * u + d * v)
    }

    /// The axis-aligned box `r` covers once turned about its centre.
    pub fn bounds(&self, r: Rect) -> Rect {
        let (bw, bh) = self.extent(r.w, r.h);
        Rect::new(r.x + (r.w - bw) / 2.0, r.y + (r.h - bh) / 2.0, bw, bh)
    }

    /// The width and height of the box a `w` × `h` rectangle covers once turned.
    pub fn extent(&self, w: f32, h: f32) -> (f32, f32) {
        let (w, h) = (finite(w).abs(), finite(h).abs());
        let r = normalize_degrees(self.deg).to_radians();
        let (s, c) = (r.sin().abs(), r.cos().abs());
        (w * c + h * s, w * s + h * c)
    }

    /// This spin applied inside `outer` (a group member's own spin within its group's): the one
    /// spin doing both.
    pub fn within(self, outer: Spin) -> Spin {
        // outer = R(θo)·Fo, self = R(θs)·Fs; Fo·R(θ) = R(±θ)·Fo (one mirror reverses the turn).
        let one_mirror = outer.flip_h != outer.flip_v;
        let inner = if one_mirror { -self.deg } else { self.deg };
        Spin::new(outer.deg + inner, outer.flip_h != self.flip_h, outer.flip_v != self.flip_v)
    }
}

/// Standard paper sizes (Word's Layout › Size gallery), in points (portrait).
pub const PAPER_SIZES: &[(&str, f32, f32)] = &[
    ("Letter", 612.0, 792.0),
    ("Legal", 612.0, 1008.0),
    ("Executive", 522.0, 756.0),
    ("A3", 841.89, 1190.55),
    ("A4", 595.28, 841.89),
    ("A5", 419.53, 595.28),
    ("B4 (JIS)", 728.5, 1031.8),
    ("B5 (JIS)", 515.9, 728.5),
    ("Tabloid", 792.0, 1224.0),
    ("Statement", 396.0, 612.0),
    ("Envelope #10", 297.0, 684.0),
    ("Envelope DL", 311.8, 623.6),
];

/// The paper size whose dimensions match (either orientation, within half a point).
pub fn paper_name(w: f32, h: f32) -> Option<&'static str> {
    PAPER_SIZES
        .iter()
        .find(|(_, pw, ph)| ((pw - w).abs() < 0.5 && (ph - h).abs() < 0.5) || ((pw - h).abs() < 0.5 && (ph - w).abs() < 0.5))
        .map(|(n, _, _)| *n)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unit_round_trips() {
        assert_eq!(twips(1440), 72.0);
        assert_eq!(to_twips(72.0), 1440);
        assert_eq!(emu(914_400), 72.0);
        assert_eq!(to_emu(72.0), 914_400);
        assert_eq!(half_points(24), 12.0);
        assert_eq!(to_half_points(10.5), 21);
        assert_eq!(to_twips(f32::NAN), 0);
    }

    #[test]
    fn parses_measurements() {
        assert_eq!(parse_measure("1\"", Unit::Points), Some(72.0));
        assert_eq!(parse_measure("1 in", Unit::Points), Some(72.0));
        assert_eq!(parse_measure("12pt", Unit::Inches), Some(12.0));
        assert_eq!(parse_measure("2", Unit::Inches), Some(144.0));
        assert_eq!(parse_measure("1,5 li", Unit::Inches), Some(18.0));
        let cm = parse_measure("2.54 cm", Unit::Inches).unwrap();
        assert!((cm - 72.0).abs() < 0.01);
        assert_eq!(parse_measure("abc", Unit::Inches), None);
        assert_eq!(parse_measure("", Unit::Inches), None);
        assert_eq!(parse_measure("1 furlong", Unit::Inches), None);
    }

    #[test]
    fn formats_measurements() {
        assert_eq!(Unit::Inches.format(72.0), "1\"");
        assert_eq!(Unit::Inches.format(90.0), "1.25\"");
        assert_eq!(Unit::Points.format(12.0), "12 pt");
        assert_eq!(Unit::Centimeters.format(72.0), "2.54 cm");
    }

    #[test]
    fn rect_ops() {
        let a = Rect::new(0.0, 0.0, 10.0, 10.0);
        let b = Rect::new(5.0, 5.0, 10.0, 10.0);
        assert!(a.intersects(&b));
        assert_eq!(a.union(&b), Rect::new(0.0, 0.0, 15.0, 15.0));
        assert!(a.contains(Point::new(5.0, 5.0)));
        assert_eq!(Rect::new(0.0, 0.0, -5.0, f32::NAN).w, 0.0);
        assert_eq!(a.inset(1.0, 2.0, 3.0, 4.0), Rect::new(1.0, 2.0, 6.0, 4.0));
    }

    #[test]
    fn paper_names() {
        assert_eq!(paper_name(612.0, 792.0), Some("Letter"));
        assert_eq!(paper_name(792.0, 612.0), Some("Letter"));
        assert_eq!(paper_name(100.0, 100.0), None);
    }
}
