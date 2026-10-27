use super::*;

/// How an area is painted: one colour or a gradient (page points), opacities 0..1.
#[derive(Clone, Debug, PartialEq)]
pub enum Paint {
    Solid(Rgb, f32),
    /// A linear gradient from `p0` to `p1`; stops are (offset 0..1, colour, opacity).
    Linear {
        p0: (f32, f32),
        p1: (f32, f32),
        stops: Vec<(f32, Rgb, f32)>,
    },
    /// A circular gradient out from `c` to radius `r`; stops as [`Paint::Linear`]'s.
    Radial {
        c: (f32, f32),
        r: f32,
        stops: Vec<(f32, Rgb, f32)>,
    },
}

impl Paint {
    /// Opaque `c`.
    pub fn solid(c: Rgb) -> Paint {
        Paint::Solid(c, 1.0)
    }
    /// This paint with every opacity times `a` (soft edges' bands).
    pub fn faded(&self, a: f32) -> Paint {
        let fade = |s: &Vec<(f32, Rgb, f32)>| s.iter().map(|(o, c, x)| (*o, *c, x * a)).collect();
        match self {
            Paint::Solid(c, x) => Paint::Solid(*c, x * a),
            Paint::Linear { p0, p1, stops } => Paint::Linear { p0: *p0, p1: *p1, stops: fade(stops) },
            Paint::Radial { c, r, stops } => Paint::Radial { c: *c, r: *r, stops: fade(stops) },
        }
    }
    /// The colour and opacity this paint gives the point (`x`, `y`): stops blended, padded past
    /// the ends.
    pub fn at(&self, x: f32, y: f32) -> (Rgb, f32) {
        let (stops, t) = match self {
            Paint::Solid(c, a) => return (*c, *a),
            Paint::Linear { p0, p1, stops } => {
                let d = (p1.0 - p0.0, p1.1 - p0.1);
                (stops, ((x - p0.0) * d.0 + (y - p0.1) * d.1) / (d.0 * d.0 + d.1 * d.1).max(1e-6))
            }
            Paint::Radial { c, r, stops } => (stops, (x - c.0).hypot(y - c.1) / r.max(1e-6)),
        };
        let (Some(first), Some(last)) = (stops.first(), stops.last()) else { return (Rgb::BLACK, 0.0) };
        let t = if t.is_finite() { t } else { 0.0 };
        let Some(i) = stops.iter().position(|s| s.0 >= t) else { return (last.1, last.2) };
        let Some(a) = i.checked_sub(1).and_then(|j| stops.get(j)) else { return (first.1, first.2) };
        let b = &stops[i];
        let f = if b.0 > a.0 { (t - a.0) / (b.0 - a.0) } else { 0.0 };
        let mix = |x: u8, y: u8| (f32::from(x) + (f32::from(y) - f32::from(x)) * f).round().clamp(0.0, 255.0) as u8;
        (Rgb(mix(a.1.0, b.1.0), mix(a.1.1, b.1.1), mix(a.1.2, b.1.2)), a.2 + (b.2 - a.2) * f)
    }
}

/// How shape fill `fill` paints the shape's frame `frame` (#1500). A linear gradient runs across
/// the frame at its angle (`scaled`: the angle of the frame squashed to a square); a path
/// gradient runs from the centre of its focus rectangle out to the farthest corner, drawn
/// circular whatever its outline (rectangular and shape-following ones are approximated).
/// `tileRect` and `flip` are kept but not drawn. `None` for no fill.
pub fn paint_for(fill: &wordcraft_doc::fill::Fill, frame: Rect) -> Option<Paint> {
    use wordcraft_doc::fill::{Fill, GradientKind};
    let g = match fill.sanitized()? {
        Fill::Solid { color, transparency } => return Some(Paint::Solid(color, 1.0 - transparency / 100.0)),
        Fill::Gradient(g) => g,
    };
    let stops = g.stops.iter().map(|s| (s.pos / 100.0, s.color, 1.0 - s.transparency / 100.0)).collect();
    let (cx, cy) = (frame.x + frame.w / 2.0, frame.y + frame.h / 2.0);
    Some(match g.kind {
        GradientKind::Linear { angle, scaled } => {
            let (s, c) = angle.to_radians().sin_cos();
            // Scaled: the isolines of the square's gradient stretched with the frame.
            let (dx, dy) = if scaled && frame.w > 0.0 && frame.h > 0.0 { (c / frame.w, s / frame.h) } else { (c, s) };
            let len = dx.hypot(dy);
            let (dx, dy) = if len > 0.0 && len.is_finite() { (dx / len, dy / len) } else { (1.0, 0.0) };
            let half = (frame.w * dx.abs() + frame.h * dy.abs()) / 2.0;
            Paint::Linear { p0: (cx - dx * half, cy - dy * half), p1: (cx + dx * half, cy + dy * half), stops }
        }
        GradientKind::Path { focus: [l, t, r, b], .. } => {
            let x0 = frame.x + frame.w * l / 100.0;
            let x1 = frame.right() - frame.w * r / 100.0;
            let y0 = frame.y + frame.h * t / 100.0;
            let y1 = frame.bottom() - frame.h * b / 100.0;
            let c = ((x0 + x1) / 2.0, (y0 + y1) / 2.0);
            let corners = [(frame.x, frame.y), (frame.right(), frame.y), (frame.x, frame.bottom()), (frame.right(), frame.bottom())];
            let r = corners.iter().map(|(x, y)| (x - c.0).hypot(y - c.1)).fold(0.0f32, f32::max);
            Paint::Radial { c, r: r.max(0.01), stops }
        }
    })
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Stroke {
    Solid,
    Dotted,
    Dashed,
    Double,
    /// Three thin lines side by side (a `triple` border).
    Triple,
    Wave,
    /// Word's border patterns (paragraph, cell and table borders): dots one width on, one off;
    /// dashes three on, one off; dash and dot.
    BorderDotted,
    BorderDashed,
    BorderDotDash,
}

impl Stroke {
    /// Dash and gap lengths, alternately, in line widths; `None` for the strokes that aren't
    /// dashed.
    pub fn dashes(self) -> Option<&'static [f32]> {
        match self {
            Stroke::Dotted => Some(&[1.0, 2.0]),
            Stroke::Dashed => Some(&[4.0, 2.0]),
            Stroke::BorderDotted => Some(&[1.0, 1.0]),
            Stroke::BorderDashed => Some(&[3.0, 1.0]),
            Stroke::BorderDotDash => Some(&[3.0, 1.0, 1.0, 1.0]),
            Stroke::Solid | Stroke::Double | Stroke::Triple | Stroke::Wave => None,
        }
    }
}
