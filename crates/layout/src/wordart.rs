//! WordArt drawing: text effects as paint layers, and text warps.
//!
//! Text with effects is drawn from its glyph outlines ([`glyph_path`]) as a [`Draw::Art`]
//! (crate::display::Draw::Art), which [`art_layers`] turns into plain fills and strokes for the
//! raster and PDF back ends: reflection, shadow, glow, fill, outline, in that order. A warped
//! text box's outlines are bent by [`Warp`] before that. Blurs are approximated with
//! [`wordcraft_doc::effects::bands`], as for shapes.

use kurbo::{Affine, BezPath, PathEl, Point, Shape};
use wordcraft_doc::props::Rgb;
use wordcraft_doc::wordart::{TextEffects, TextFill, TextWarp};
use wordcraft_fonts::FaceRef;
use wordcraft_geom::Rect;

/// How a layer is painted.
#[derive(Clone, Debug, PartialEq)]
pub enum ArtPaint {
    Solid(Rgb, f32),
    /// A linear gradient from `p0` to `p1` (page points); stops are (offset 0..1, colour, opacity).
    Linear {
        p0: (f32, f32),
        p1: (f32, f32),
        stops: Vec<(f32, Rgb, f32)>,
    },
}

/// One fill (`stroke: None`) or stroke (its width, points) of a path.
#[derive(Clone, Debug)]
pub struct ArtLayer {
    pub path: BezPath,
    pub paint: ArtPaint,
    pub stroke: Option<f32>,
}

/// The outlines of `glyphs` (glyph id, pen x, baseline y in page points) of `face` at `size`, as
/// one path in page coordinates (slanted for synthetic italics).
pub fn glyph_path(face: &FaceRef, size: f32, glyphs: &[(u32, f32, f32)], synth_italic: bool) -> BezPath {
    let db = wordcraft_fonts::FontDb::global();
    let k = f64::from(size) / face.upem.max(1.0);
    let skew = if synth_italic { Affine::new([1.0, 0.0, -0.21, 1.0, 0.0, 0.0]) } else { Affine::IDENTITY };
    let mut out = BezPath::new();
    for &(gid, x, y) in glyphs {
        if !(x.is_finite() && y.is_finite()) {
            continue;
        }
        let m = Affine::translate((f64::from(x), f64::from(y))) * skew * Affine::scale(k);
        for el in db.outline(face, gid).elements() {
            out.push(m * *el);
        }
    }
    out
}

fn bbox(path: &BezPath) -> Option<kurbo::Rect> {
    if path.elements().is_empty() {
        return None;
    }
    let b = path.bounding_box();
    (b.x0.is_finite() && b.y0.is_finite() && b.x1.is_finite() && b.y1.is_finite()).then_some(b)
}

/// The layers that draw `path` (text outlines, page coordinates) with effects `fx` (sanitized
/// here); `color` is the run's own colour (the fill when `fx` sets none), `alpha` an overall
/// opacity.
pub fn art_layers(path: &BezPath, fx: &TextEffects, color: Rgb, alpha: f32) -> Vec<ArtLayer> {
    let fx = fx.sanitized();
    let alpha = if alpha.is_finite() { alpha.clamp(0.0, 1.0) } else { 1.0 };
    let Some(b) = bbox(path) else { return Vec::new() };
    let mut out = Vec::new();
    let fill = fx.fill.clone().unwrap_or(TextFill::Solid { color, transparency: 0.0 });
    let fill_paint = |path: &BezPath| -> Option<ArtPaint> {
        match &fill {
            TextFill::None => None,
            TextFill::Solid { color, transparency } => Some(ArtPaint::Solid(*color, alpha * (1.0 - transparency / 100.0))),
            TextFill::Gradient { stops, angle } => {
                let b = bbox(path)?;
                let (s, c) = (f64::from(*angle).to_radians().sin(), f64::from(*angle).to_radians().cos());
                let d = (b.width() * c.abs() + b.height() * s.abs()) / 2.0;
                let ctr = b.center();
                let p0 = ((ctr.x - d * c) as f32, (ctr.y - d * s) as f32);
                let p1 = ((ctr.x + d * c) as f32, (ctr.y + d * s) as f32);
                Some(ArtPaint::Linear {
                    p0,
                    p1,
                    stops: stops.iter().map(|g| (g.pos / 100.0, g.color, alpha * (1.0 - g.transparency / 100.0))).collect(),
                })
            }
        }
    };
    let outline = fx.outline.and_then(|o| o.color.map(|c| (c, o.width.max(0.25), alpha * (1.0 - o.transparency / 100.0))));
    // A colour standing for the letters (shadows and reflections of hollow text use the outline's).
    let ink = fill.main_color().or(outline.map(|o| o.0)).unwrap_or(color);
    if let Some(r) = fx.reflection {
        // Mirrored about the letters' bottom edge, `distance` below it, fading out downwards.
        let gap = f64::from(r.distance);
        let m = Affine::new([1.0, 0.0, 0.0, -1.0, 0.0, 2.0 * b.y1 + gap]);
        let mirror = m * path.clone();
        let top = (b.y1 + gap) as f32;
        let bottom = top + (b.height() as f32) * (r.size / 100.0).max(0.01);
        let x = b.x0 as f32;
        let a = alpha * (1.0 - r.transparency / 100.0);
        out.push(ArtLayer {
            path: mirror,
            paint: ArtPaint::Linear { p0: (x, top), p1: (x, bottom), stops: vec![(0.0, ink, a), (1.0, ink, 0.0)] },
            stroke: None,
        });
    }
    if let Some(sh) = fx.shadow {
        let (dx, dy) = sh.offset();
        let moved = Affine::translate((f64::from(dx), f64::from(dy))) * path.clone();
        for (g, a) in wordcraft_doc::effects::bands(0.0, sh.blur / 2.0, sh.opacity() * alpha) {
            let paint = ArtPaint::Solid(sh.color, a);
            out.push(ArtLayer { path: moved.clone(), paint: paint.clone(), stroke: None });
            if g > 0.05 {
                out.push(ArtLayer { path: moved.clone(), paint, stroke: Some(2.0 * g) });
            }
        }
    }
    if let Some(gl) = fx.glow {
        for (g, a) in wordcraft_doc::effects::bands(0.0, gl.size, gl.opacity() * alpha) {
            if g > 0.05 {
                out.push(ArtLayer { path: path.clone(), paint: ArtPaint::Solid(gl.color, a), stroke: Some(2.0 * g) });
            }
        }
    }
    if let Some(p) = fill_paint(path) {
        out.push(ArtLayer { path: path.clone(), paint: p, stroke: None });
    }
    if let Some((c, w, a)) = outline {
        out.push(ArtLayer { path: path.clone(), paint: ArtPaint::Solid(c, a), stroke: Some(w) });
    }
    out
}

/// How a warp bends text: two edges (top and bottom, the text stretched between them), or an arc
/// (the text running along an ellipse, its letters standing out or in).
#[derive(Clone, Copy, Debug)]
enum Bend {
    /// The top and bottom edges' heights (fractions of the frame, 0 = top) at `u` across.
    Edges(fn(f32, f32) -> (f32, f32)),
    /// Along an ellipse from angle `start` (radians, counter-clockwise from the right, y up)
    /// sweeping `sweep` (negative: clockwise); `outward`: the letters' tops point away from the
    /// centre.
    Arc { start: f32, sweep: f32, outward: bool },
}

/// A warp fitted to a frame: maps the text's box (`u`, `v` in 0..1, `v` = 0 at the top) onto the
/// frame.
#[derive(Clone, Copy, Debug)]
pub struct Warp {
    frame: Rect,
    bend: Bend,
    /// How far the edges bend (a fraction of the frame's height).
    k: f32,
    /// The text's thickness along an arc, as a fraction of the radius.
    thick: f32,
}

fn slant_up(u: f32, k: f32) -> (f32, f32) {
    (k * (1.0 - u), 1.0 - k * u)
}
fn slant_down(u: f32, k: f32) -> (f32, f32) {
    (k * u, 1.0 - k * (1.0 - u))
}
fn inflate(u: f32, k: f32) -> (f32, f32) {
    let s = (std::f32::consts::PI * u).sin();
    (k * (1.0 - s), 1.0 - k * (1.0 - s))
}
fn deflate(u: f32, k: f32) -> (f32, f32) {
    let s = (std::f32::consts::PI * u).sin();
    (k * s, 1.0 - k * s)
}
fn wave(u: f32, k: f32) -> (f32, f32) {
    let d = k * (2.0 * std::f32::consts::PI * u).sin();
    (k - d, 1.0 - k - d)
}
fn chevron_up(u: f32, k: f32) -> (f32, f32) {
    let t = (2.0 * u - 1.0).abs();
    (k * t, 1.0 - k * (1.0 - t))
}
fn chevron_down(u: f32, k: f32) -> (f32, f32) {
    let t = (2.0 * u - 1.0).abs();
    (k * (1.0 - t), 1.0 - k * t)
}
fn triangle_up(u: f32, k: f32) -> (f32, f32) {
    (k * (2.0 * u - 1.0).abs(), 1.0)
}
fn triangle_down(u: f32, k: f32) -> (f32, f32) {
    (0.0, 1.0 - k * (2.0 * u - 1.0).abs())
}

impl Warp {
    /// The warp `w` fitted to `frame` for text whose box is `aspect` times as tall as it is
    /// wide. `None` for warps drawn flat (plain, and those WordCraft doesn't bend).
    pub fn new(w: &TextWarp, frame: Rect, aspect: f32) -> Option<Warp> {
        use std::f32::consts::PI;
        if !(frame.w > 0.5 && frame.h > 0.5 && frame.w.is_finite() && frame.h.is_finite()) {
            return None;
        }
        // `adj` as a fraction (100000ths) of the frame's height, within `max`, else `dflt`.
        let frac = |dflt: f32, max: f32| w.adj_value("adj").map_or(dflt, |v| (v as f32 / 100_000.0).clamp(0.0, max));
        // `adj` as the angle the arc sweeps (60000ths of a degree), else a half turn.
        let sweep = w.adj_value("adj").map_or(PI, |v| (v as f32 / 60_000.0).clamp(10.0, 350.0).to_radians());
        let edges = |f: fn(f32, f32) -> (f32, f32), k: f32| (Bend::Edges(f), k);
        let (bend, k) = match w.preset.as_str() {
            "textSlantUp" => edges(slant_up, frac(0.4, 0.9)),
            "textSlantDown" => edges(slant_down, frac(0.4, 0.9)),
            "textInflate" => edges(inflate, frac(0.3, 0.45)),
            "textDeflate" => edges(deflate, frac(0.3, 0.45)),
            "textWave1" => edges(wave, frac(0.125, 0.3)),
            "textChevron" => edges(chevron_up, frac(0.4, 0.9)),
            "textChevronInverted" => edges(chevron_down, frac(0.4, 0.9)),
            "textTriangle" => edges(triangle_up, frac(0.5, 0.9)),
            "textTriangleInverted" => edges(triangle_down, frac(0.5, 0.9)),
            "textArchUp" => (Bend::Arc { start: PI / 2.0 + sweep / 2.0, sweep: -sweep, outward: true }, 0.0),
            "textArchDown" => (Bend::Arc { start: 1.5 * PI - sweep / 2.0, sweep, outward: false }, 0.0),
            "textCircle" => (Bend::Arc { start: 1.5 * PI - 0.02, sweep: -(2.0 * PI - 0.04), outward: true }, 0.0),
            _ => return None,
        };
        // Along an arc the text keeps its proportions: as thick as its length along the mean
        // radius makes it, within reason.
        let thick = match bend {
            Bend::Arc { sweep, .. } => {
                let aspect = if aspect.is_finite() { aspect.clamp(0.0, 10.0) } else { 0.2 };
                (aspect * sweep.abs()).clamp(0.08, 0.6)
            }
            Bend::Edges(_) => 0.0,
        };
        Some(Warp { frame, bend, k, thick })
    }

    /// The page point for text box point (`u`, `v`).
    pub fn map(&self, u: f32, v: f32) -> (f32, f32) {
        let (u, v) = (if u.is_finite() { u } else { 0.0 }, if v.is_finite() { v } else { 0.0 });
        let f = self.frame;
        match self.bend {
            Bend::Edges(edge) => {
                let (top, bottom) = edge(u.clamp(-1.0, 2.0), self.k);
                (f.x + u * f.w, f.y + (top + (bottom - top) * v) * f.h)
            }
            Bend::Arc { start, sweep, outward } => {
                let (rx, ry) = (f.w / 2.0, f.h / 2.0);
                let (cx, cy) = (f.x + rx, f.y + ry);
                let a = start + sweep * u;
                let rho = if outward { 1.0 - v * self.thick } else { 1.0 - self.thick + v * self.thick };
                (cx + rx * rho * a.cos(), cy - ry * rho * a.sin())
            }
        }
    }
}

/// `path` bent by `warp`, its box `src` mapped onto the warp's frame. Curves are flattened and
/// long lines cut short so they bend smoothly.
pub fn warp_path(path: &BezPath, src: kurbo::Rect, warp: &Warp) -> BezPath {
    let (w, h) = (src.width().max(1e-3), src.height().max(1e-3));
    let step = (w / 96.0).max(0.25);
    let to = |p: Point| {
        let (x, y) = warp.map(((p.x - src.x0) / w) as f32, ((p.y - src.y0) / h) as f32);
        Point::new(f64::from(x), f64::from(y))
    };
    let mut out = BezPath::new();
    let mut last: Option<Point> = None;
    // Bounded: a hostile path can't make it endless (each line cut in at most 256 pieces).
    kurbo::flatten(path.elements().iter().copied(), 0.05, |el| match el {
        PathEl::MoveTo(p) => {
            out.move_to(to(p));
            last = Some(p);
        }
        PathEl::LineTo(p) => {
            if let Some(a) = last {
                let n = ((a.distance(p) / step).ceil() as usize).clamp(1, 256);
                for i in 1..=n {
                    out.line_to(to(a.lerp(p, i as f64 / n as f64)));
                }
            } else {
                out.move_to(to(p));
            }
            last = Some(p);
        }
        PathEl::ClosePath => out.close_path(),
        _ => {}
    });
    out
}

/// The box of `path`, if it has one.
pub fn path_box(path: &BezPath) -> Option<kurbo::Rect> {
    bbox(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn arch_baselines_lie_on_the_arc() {
        let frame = Rect::new(10.0, 20.0, 200.0, 100.0);
        let w = Warp::new(&TextWarp::new("textArchUp").unwrap(), frame, 0.25).unwrap();
        let (cx, cy, rx, ry) = (110.0, 70.0, 100.0, 50.0);
        // A baseline (fixed v) maps onto one ellipse around the frame's centre, over the top.
        for v in [0.0, 0.8] {
            let rho: Vec<f32> = (0..=10)
                .map(|i| {
                    let (x, y) = w.map(i as f32 / 10.0, v);
                    assert!(y <= cy + 1e-3, "over the top: {y}");
                    (((x - cx) / rx).powi(2) + ((y - cy) / ry).powi(2)).sqrt()
                })
                .collect();
            assert!(rho.iter().all(|r| (r - rho[0]).abs() < 1e-3), "{rho:?}");
        }
        // Left to right, the middle at the top.
        let (x0, _) = w.map(0.0, 0.0);
        let (xm, ym) = w.map(0.5, 0.0);
        let (x1, _) = w.map(1.0, 0.0);
        assert!(x0 < xm && xm < x1 && (ym - 20.0).abs() < 1e-3);
        // Arch down runs along the bottom.
        let d = Warp::new(&TextWarp::new("textArchDown").unwrap(), frame, 0.25).unwrap();
        assert!(d.map(0.5, 1.0).1 > 119.0);
    }

    #[test]
    fn flat_and_unknown_warps_draw_flat() {
        let frame = Rect::new(0.0, 0.0, 100.0, 50.0);
        assert!(Warp::new(&TextWarp::new("textPlain").unwrap(), frame, 0.2).is_none());
        assert!(Warp::new(&TextWarp::new("textCanUp").unwrap(), frame, 0.2).is_none());
        // Hostile adjust values stay within the frame.
        let mut t = TextWarp::new("textSlantUp").unwrap();
        t.set_adj("adj", i64::MAX);
        let w = Warp::new(&t, frame, f32::NAN).unwrap();
        for (u, v) in [(0.0, 0.0), (1.0, 1.0), (f32::NAN, 0.5)] {
            let (x, y) = w.map(u, v);
            assert!((0.0..=100.0).contains(&x) && (0.0..=50.0).contains(&y), "{x} {y}");
        }
    }
}
