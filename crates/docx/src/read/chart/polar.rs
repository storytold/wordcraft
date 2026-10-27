use super::canvas::{Rect, clip_text, fill, stroke, text, text_w, widest};
use super::format::format_value;
use super::model::{DataLabel, Kind, Series};
use super::plot::{Axes, has_values};
use super::*;

pub(super) fn pie(out: &mut Vec<GraphicItem>, c: &Chart, area: Rect) {
    let Some(s) = c.series.first() else { return };
    let vals: Vec<f64> = s.values.iter().map(|v| v.unwrap_or(0.0).max(0.0)).collect();
    let total: f64 = vals.iter().sum();
    let r = (area.w.min(area.h) / 2.0 - 2.0) as f64;
    if !(total > 0.0 && total.is_finite() && r >= 1.0) {
        return;
    }
    let (cx, cy) = ((area.x + area.w / 2.0) as f64, (area.y + area.h / 2.0) as f64);
    let inner = if c.kind == Kind::Doughnut { r * c.hole as f64 } else { 0.0 };
    let mut a = c.first_angle.to_radians();
    let mut labels = Vec::new();
    for (i, v) in vals.iter().enumerate() {
        if *v <= 0.0 {
            continue;
        }
        let b = a + v / total * TAU;
        let mut segs = Vec::new();
        if inner > 0.0 {
            segs.push(pt(polar(cx, cy, r, a)));
            arc(&mut segs, (cx, cy, r), a, b);
            segs.push(line_to(polar(cx, cy, inner, b)));
            arc(&mut segs, (cx, cy, inner), b, a);
        } else {
            segs.push(pt((cx, cy)));
            segs.push(line_to(polar(cx, cy, r, a)));
            arc(&mut segs, (cx, cy, r), a, b);
        }
        segs.push(PathSeg::Close);
        fill(out, segs, c.point_colour(0, i));
        if let Some(text) = label_of(s, i) {
            // Halfway across the ring (pie: two thirds out from the centre).
            let at = if inner > 0.0 { (r + inner) / 2.0 } else { r * 0.65 };
            let (x, y) = polar(cx, cy, at, (a + b) / 2.0);
            labels.push(DataLabel { x: x as f32, y: y as f32, text, inside: true });
        }
        a = b;
    }
    draw_data_labels(out, &labels, c.text_size, false);
}

/// A radar chart in `area`: category spokes from the centre, concentric grid polygons at the value
/// ticks, one polygon per series (filled, or outlined with markers) and the category labels at the
/// spoke ends. The lowest tick is the centre.
pub(super) fn radar(out: &mut Vec<GraphicItem>, c: &Chart, area: Rect) {
    const MARKER: f32 = 5.0;
    if c.n == 0 {
        return;
    }
    let fs = c.text_size;
    let lh = fs * LINE;
    let labelled = c.cats.iter().any(|s| !s.is_empty());
    let side = if labelled { widest(&c.cats, fs).min(area.w * 0.25) + GAP + 2.0 } else { 0.0 };
    let vert = if labelled { lh + GAP } else { 0.0 };
    let r = ((area.w / 2.0 - side).min(area.h / 2.0 - vert) - 2.0) as f64;
    if r.is_nan() || r < 8.0 {
        return;
    }
    let (cx, cy) = ((area.x + area.w / 2.0) as f64, (area.y + area.h / 2.0) as f64);
    let vs = Axes::of(c).vs;
    let span = vs.hi - vs.lo;
    let angle = |i: usize| i as f64 / c.n as f64 * TAU;
    let at = |v: f64, i: usize| {
        let t = if span > 0.0 { ((v - vs.lo) / span).clamp(0.0, 1.0) } else { 0.0 };
        polar(cx, cy, r * t, angle(i))
    };
    let ticks = vs.ticks();
    if c.val.grid {
        for t in ticks.iter().skip(1) {
            let mut segs: Vec<PathSeg> = (0..c.n).map(|i| if i == 0 { pt(at(*t, i)) } else { line_to(at(*t, i)) }).collect();
            segs.push(PathSeg::Close);
            stroke(out, segs, GRID, 0.75);
        }
    }
    for i in 0..c.n {
        let end = polar(cx, cy, r, angle(i));
        stroke(out, vec![pt((cx, cy)), line_to(end)], GRID, 0.75);
    }
    let mut labels = Vec::new();
    for (j, s) in c.series.iter().enumerate().filter(|(_, s)| has_values(s)) {
        let col = c.series_colour(j);
        // A missing value counts as the centre, as Word draws it.
        let ring: Vec<(f64, f64)> = (0..c.n).map(|i| at(s.values.get(i).copied().flatten().unwrap_or(vs.lo), i)).collect();
        let mut segs: Vec<PathSeg> = ring.iter().enumerate().map(|(i, p)| if i == 0 { pt(*p) } else { line_to(*p) }).collect();
        segs.push(PathSeg::Close);
        if c.radar_fill {
            fill(out, segs.clone(), col);
        }
        stroke(out, segs, col, s.line_w);
        for (i, p) in ring.iter().enumerate() {
            if c.radar_marks && s.values.get(i).copied().flatten().is_some() {
                let rect = [p.0 as f32 - MARKER / 2.0, p.1 as f32 - MARKER / 2.0, MARKER, MARKER];
                out.push(GraphicItem::Shape { rect, kind: ShapeKind::Ellipse, fill: Some(col), stroke: None, stroke_width: 0.0 });
            }
            if let Some(text) = label_of(s, i) {
                labels.push(DataLabel { x: p.0 as f32, y: p.1 as f32, text, inside: false });
            }
        }
    }
    draw_data_labels(out, &labels, fs, false);
    if !c.val.deleted {
        for t in &ticks {
            let y = at(*t, 0).1 as f32;
            let s = format_value(*t, &c.val_fmt, vs.step);
            let w = text_w(&s, fs) + 4.0;
            text(out, Rect { x: cx as f32 - GAP / 2.0 - w, y: y - lh / 2.0, w, h: lh }, &s, fs, TextAlign::Right, false);
        }
    }
    if labelled {
        for (i, s) in c.cats.iter().enumerate().filter(|(_, s)| !s.is_empty()) {
            let a = angle(i);
            let (x, y) = polar(cx, cy, r + GAP as f64, a);
            let (x, y) = (x as f32, y as f32);
            let w = side - GAP;
            let txt = clip_text(s, w, fs);
            let (sx, sy) = (a.sin(), a.cos());
            let (rect, align) = if sx > 0.2 {
                (Rect { x, y: y - lh / 2.0, w, h: lh }, TextAlign::Left)
            } else if sx < -0.2 {
                (Rect { x: x - w, y: y - lh / 2.0, w, h: lh }, TextAlign::Right)
            } else {
                let cw = (text_w(&txt, fs) + 4.0).max(1.0);
                (Rect { x: x - cw / 2.0, y: if sy > 0.0 { y - lh } else { y }, w: cw, h: lh }, TextAlign::Center)
            };
            text(out, rect, &txt, fs, align, false);
        }
    }
}

/// The data label of point `i` of series `s`, when it shows one: the value as the file has it,
/// in the series' number format.
pub(super) fn label_of(s: &Series, i: usize) -> Option<String> {
    if s.labels != Some(true) {
        return None;
    }
    let v = s.raw.get(i).copied().flatten()?;
    let fmt = s.fmt.as_deref().filter(|f| !f.eq_ignore_ascii_case("General"));
    Some(match fmt {
        Some(f) => format_value(v, f, 1.0),
        None => general_value(v),
    })
}

/// A value in the `General` format: whole numbers without decimals, others with up to six.
fn general_value(v: f64) -> String {
    if v.fract() == 0.0 && v.abs() < 1e15 {
        return format!("{v:.0}");
    }
    let s = format!("{v:.6}");
    let s = s.trim_end_matches('0').trim_end_matches('.');
    if s == "-0" { "0".to_string() } else { s.to_string() }
}

/// Data labels over the drawn chart. Beside labels sit above their point, or right of it for
/// horizontal bars (`beside_right`).
pub(super) fn draw_data_labels(out: &mut Vec<GraphicItem>, labels: &[DataLabel], fs: f32, beside_right: bool) {
    let lh = fs * LINE;
    for l in labels {
        let w = text_w(&l.text, fs) + 4.0;
        let (rect, align) = if l.inside {
            (Rect { x: l.x - w / 2.0, y: l.y - lh / 2.0, w, h: lh }, TextAlign::Center)
        } else if beside_right {
            (Rect { x: l.x + GAP / 2.0, y: l.y - lh / 2.0, w, h: lh }, TextAlign::Left)
        } else {
            (Rect { x: l.x - w / 2.0, y: l.y - lh, w, h: lh }, TextAlign::Center)
        };
        let color = if l.inside { Rgb::WHITE } else { TEXT };
        out.push(GraphicItem::Text { rect: rect.arr(), text: l.text.clone(), size: fs, color, bold: false, align, font: None });
    }
}

/// A point at clockwise angle `t` (radians from 12 o'clock) on a circle of radius `r`.
fn polar(cx: f64, cy: f64, r: f64, t: f64) -> (f64, f64) {
    (cx + r * t.sin(), cy - r * t.cos())
}

fn pt(p: (f64, f64)) -> PathSeg {
    PathSeg::Move(p.0 as f32, p.1 as f32)
}

fn line_to(p: (f64, f64)) -> PathSeg {
    PathSeg::Line(p.0 as f32, p.1 as f32)
}

/// Cubic segments along a circle from angle `a` to `b` (the current point is at `a`). Each piece
/// spans at most a quarter turn.
fn arc(segs: &mut Vec<PathSeg>, (cx, cy, r): (f64, f64, f64), a: f64, b: f64) {
    let n = (((b - a).abs() / FRAC_PI_2).ceil() as usize).clamp(1, 16);
    let d = (b - a) / n as f64;
    let k = 4.0 / 3.0 * (d / 4.0).tan();
    for j in 0..n {
        let t0 = a + d * j as f64;
        let t1 = t0 + d;
        let p0 = polar(cx, cy, r, t0);
        let p1 = polar(cx, cy, r, t1);
        let c1 = (p0.0 + k * r * t0.cos(), p0.1 + k * r * t0.sin());
        let c2 = (p1.0 - k * r * t1.cos(), p1.1 - k * r * t1.sin());
        segs.push(PathSeg::Cubic(c1.0 as f32, c1.1 as f32, c2.0 as f32, c2.1 as f32, p1.0 as f32, p1.1 as f32));
    }
}
