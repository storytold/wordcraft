//! Shape effects (DrawingML `a:effectLst`, ECMA-376 Part 1 §20.1.8): an outer shadow, a glow and
//! soft edges. Lengths are points, angles degrees, transparencies percent (0 = opaque).
//!
//! Renderers approximate the blurs with [`bands`]: a few copies of the shape's silhouette, grown
//! or shrunk by steps and overlaid with partial opacity, so the screen and PDF output (which has
//! no blur filter) match.

use serde::{Deserialize, Serialize};

use crate::props::Rgb;

/// Largest blur radius, points.
pub const MAX_BLUR: f32 = 100.0;
/// Largest shadow distance, points.
pub const MAX_DISTANCE: f32 = 200.0;
/// Largest glow size, points.
pub const MAX_GLOW: f32 = 150.0;
/// Largest soft-edge radius, points.
pub const MAX_SOFT_EDGE: f32 = 100.0;

/// The effects on a shape. Empty (the default) draws the shape plainly.
#[derive(Clone, Copy, Debug, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ShapeEffects {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shadow: Option<Shadow>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub glow: Option<Glow>,
    /// Soft-edge radius, points: the shape fades out over this distance inside its outline.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub soft_edge: Option<f32>,
}

/// An outer shadow: a blurred copy of the shape's silhouette cast `distance` points away in
/// direction `angle` (degrees clockwise from the positive x axis, y pointing down).
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Shadow {
    pub color: Rgb,
    pub transparency: f32,
    pub blur: f32,
    pub distance: f32,
    pub angle: f32,
}

impl Default for Shadow {
    /// "Offset: Bottom Right".
    fn default() -> Self {
        Shadow { color: Rgb::BLACK, transparency: 60.0, blur: 4.0, distance: 3.0, angle: 45.0 }
    }
}

/// A glow: a soft band of colour `size` points wide around the shape's outline.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Glow {
    pub color: Rgb,
    pub size: f32,
    pub transparency: f32,
}

impl Default for Glow {
    fn default() -> Self {
        Glow { color: Rgb(0x15, 0x60, 0x82), size: 5.0, transparency: 60.0 }
    }
}

fn clamp(v: f32, lo: f32, hi: f32, dflt: f32) -> f32 {
    if v.is_finite() { v.clamp(lo, hi) } else { dflt }
}

/// The outer-shadow presets (no perspective shadows): id, label.
pub const SHADOW_PRESETS: [(&str, &str); 9] = [
    ("offsetBottomRight", "Offset: Bottom Right"),
    ("offsetBottom", "Offset: Bottom"),
    ("offsetBottomLeft", "Offset: Bottom Left"),
    ("offsetRight", "Offset: Right"),
    ("offsetCenter", "Offset: Center"),
    ("offsetLeft", "Offset: Left"),
    ("offsetTopRight", "Offset: Top Right"),
    ("offsetTop", "Offset: Top"),
    ("offsetTopLeft", "Offset: Top Left"),
];

impl Shadow {
    /// A preset by id (see [`SHADOW_PRESETS`]).
    pub fn preset(id: &str) -> Option<Shadow> {
        let d = Shadow::default();
        let angle = match id {
            "offsetBottomRight" => 45.0,
            "offsetBottom" => 90.0,
            "offsetBottomLeft" => 135.0,
            "offsetLeft" => 180.0,
            "offsetTopLeft" => 225.0,
            "offsetTop" => 270.0,
            "offsetTopRight" => 315.0,
            "offsetRight" => 0.0,
            "offsetCenter" => return Some(Shadow { distance: 0.0, blur: 5.0, ..d }),
            _ => return None,
        };
        Some(Shadow { angle, ..d })
    }
    /// Finite, in range.
    pub fn sanitized(self) -> Shadow {
        let angle = clamp(self.angle, -1e6, 1e6, 45.0).rem_euclid(360.0);
        Shadow {
            color: self.color,
            transparency: clamp(self.transparency, 0.0, 100.0, 60.0),
            blur: clamp(self.blur, 0.0, MAX_BLUR, 0.0),
            distance: clamp(self.distance, 0.0, MAX_DISTANCE, 0.0),
            angle: if angle.is_finite() && angle < 360.0 { angle } else { 0.0 },
        }
    }
    /// The shadow's offset from the shape, points (x right, y down).
    pub fn offset(&self) -> (f32, f32) {
        let s = self.sanitized();
        let a = s.angle.to_radians();
        (s.distance * a.cos(), s.distance * a.sin())
    }
    /// Opacity 0..1.
    pub fn opacity(&self) -> f32 {
        1.0 - self.sanitized().transparency / 100.0
    }
}

impl Glow {
    /// Finite, in range.
    pub fn sanitized(self) -> Glow {
        Glow { color: self.color, size: clamp(self.size, 0.0, MAX_GLOW, 0.0), transparency: clamp(self.transparency, 0.0, 100.0, 60.0) }
    }
    /// Opacity 0..1.
    pub fn opacity(&self) -> f32 {
        1.0 - self.sanitized().transparency / 100.0
    }
}

impl ShapeEffects {
    pub fn is_empty(&self) -> bool {
        self.shadow.is_none() && self.glow.is_none() && self.soft_edge.is_none()
    }
    /// Finite, in range; zero-size glows and soft edges dropped.
    pub fn sanitized(self) -> ShapeEffects {
        ShapeEffects {
            shadow: self.shadow.map(Shadow::sanitized),
            glow: self.glow.map(Glow::sanitized).filter(|g| g.size > 0.0),
            soft_edge: self.soft_edge.map(|r| clamp(r, 0.0, MAX_SOFT_EDGE, 0.0)).filter(|r| *r > 0.0),
        }
    }
    /// How far the effects reach beyond the shape's frame on each side (left, top, right,
    /// bottom), points — what `wp:effectExtent` records.
    pub fn extent(&self) -> [f32; 4] {
        let e = self.sanitized();
        let mut out = [0.0f32; 4];
        if let Some(g) = e.glow {
            out = [g.size; 4];
        }
        if let Some(s) = e.shadow {
            let (dx, dy) = s.offset();
            let r = s.blur / 2.0;
            let reach = [r - dx, r - dy, r + dx, r + dy];
            for (o, v) in out.iter_mut().zip(reach) {
                *o = o.max(v.max(0.0));
            }
        }
        out.map(|v| (v * 100.0).round() / 100.0)
    }
}

/// Bands approximating a blurred edge: `(grow, alpha)` pairs. Each is a copy of the silhouette
/// grown outward by `grow` points (negative shrinks it), filled with opacity `alpha`; overlaid in
/// any order, in one colour, they ramp linearly from `opacity` at `from` to nothing at `to`.
pub fn bands(from: f32, to: f32, opacity: f32) -> Vec<(f32, f32)> {
    let (from, to) = (clamp(from, -1000.0, 1000.0, 0.0), clamp(to, -1000.0, 1000.0, 0.0));
    let opacity = clamp(opacity, 0.0, 1.0, 0.0);
    if opacity <= 0.0 {
        return Vec::new();
    }
    let span = (to - from).max(0.0);
    if span < 0.25 {
        return vec![((from + to) / 2.0, opacity)];
    }
    let n = ((span / 1.5).ceil() as usize).clamp(3, 12);
    let nf = n as f32;
    (1..=n)
        .map(|m| {
            // The m-th band from the outside: after it, coverage is opacity·m/n.
            let grow = to - span * (m as f32 - 0.5) / nf;
            let before = 1.0 - opacity * (m as f32 - 1.0) / nf;
            let after = 1.0 - opacity * m as f32 / nf;
            let alpha = if before > 0.0 { 1.0 - after / before } else { 1.0 };
            (grow, alpha.clamp(0.0, 1.0))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bands_ramp_to_the_requested_opacity() {
        for (from, to, o) in [(-2.0, 2.0, 0.4), (-5.0, 0.0, 1.0), (0.0, 8.0, 0.6)] {
            let b = bands(from, to, o);
            assert!(b.iter().all(|(g, a)| *g > from && *g < to && (0.0..=1.0).contains(a)));
            let covered = 1.0 - b.iter().map(|(_, a)| 1.0 - a).product::<f32>();
            assert!((covered - o).abs() < 1e-4, "{covered} vs {o}");
        }
        assert_eq!(bands(0.0, 0.0, 0.5), vec![(0.0, 0.5)]);
        assert!(bands(f32::NAN, f32::INFINITY, f32::NAN).is_empty());
    }

    #[test]
    fn hostile_numbers_are_clamped() {
        let s = Shadow { color: Rgb::BLACK, transparency: f32::NAN, blur: 1e9, distance: -4.0, angle: -90.0 }.sanitized();
        assert_eq!((s.transparency, s.blur, s.distance, s.angle), (60.0, MAX_BLUR, 0.0, 270.0));
        let e = ShapeEffects { shadow: None, glow: Some(Glow { size: 1e9, ..Glow::default() }), soft_edge: Some(-1.0) }.sanitized();
        assert_eq!(e.glow.map(|g| g.size), Some(MAX_GLOW));
        assert_eq!(e.soft_edge, None);
        let e = ShapeEffects { glow: Some(Glow { size: f32::INFINITY, ..Glow::default() }), ..Default::default() }.sanitized();
        assert!(e.is_empty(), "a non-finite size is no glow");
        let (dx, dy) = Shadow::preset("offsetBottomRight").unwrap().offset();
        assert!(dx > 2.0 && dy > 2.0);
    }
}
