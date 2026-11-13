use super::{Fill, FillRule, LineCap, NormalizedF32, Paint, Path, PathBuilder, Rect, Rgb, ShapeKind, Stroke, Surface, Transform, krilla, rgb};

/// Largest page side we write (200", the PDF limit).
const MAX_SIDE: f32 = 14_400.0;

/// Add the segments of a kurbo path to a krilla path.
pub(crate) fn append_path(pb: &mut PathBuilder, path: &kurbo::BezPath) {
    let f = |v: f64| v as f32;
    for el in path.elements() {
        match *el {
            kurbo::PathEl::MoveTo(p) => pb.move_to(f(p.x), f(p.y)),
            kurbo::PathEl::LineTo(p) => pb.line_to(f(p.x), f(p.y)),
            kurbo::PathEl::QuadTo(a, p) => pb.quad_to(f(a.x), f(a.y), f(p.x), f(p.y)),
            kurbo::PathEl::CurveTo(a, b, p) => pb.cubic_to(f(a.x), f(a.y), f(b.x), f(b.y), f(p.x), f(p.y)),
            kurbo::PathEl::ClosePath => pb.close(),
        }
    }
}

pub(crate) fn clamp_side(v: f32) -> f32 {
    if v.is_finite() { v.clamp(3.0, MAX_SIDE) } else { 612.0 }
}

pub(crate) fn norm(v: f32) -> NormalizedF32 {
    NormalizedF32::new(if v.is_finite() { v.clamp(0.0, 1.0) } else { 1.0 }).unwrap_or(NormalizedF32::ONE)
}

pub(crate) fn fill(c: Rgb, alpha: f32) -> Fill {
    Fill { paint: rgb::Color::new(c.0, c.1, c.2).into(), opacity: norm(alpha), rule: FillRule::NonZero }
}

/// A fill in paint `p`: a colour with its opacity, or a gradient whose stops carry theirs.
pub(crate) fn paint_fill(p: &Paint) -> Fill {
    let stops = |stops: &[(f32, Rgb, f32)]| -> Vec<krilla::paint::Stop> {
        stops
            .iter()
            .map(|(o, c, a)| krilla::paint::Stop { offset: norm(*o), color: rgb::Color::new(c.0, c.1, c.2).into(), opacity: norm(*a) })
            .collect()
    };
    let paint: krilla::paint::Paint = match p {
        Paint::Solid(c, a) => return fill(*c, *a),
        Paint::Linear { p0, p1, stops: st } => krilla::paint::LinearGradient {
            x1: p0.0,
            y1: p0.1,
            x2: p1.0,
            y2: p1.1,
            transform: Transform::default(),
            spread_method: krilla::paint::SpreadMethod::Pad,
            stops: stops(st),
            anti_alias: true,
        }
        .into(),
        Paint::Radial { c, r, stops: st } => krilla::paint::RadialGradient {
            fx: c.0,
            fy: c.1,
            fr: 0.0,
            cx: c.0,
            cy: c.1,
            cr: *r,
            transform: Transform::default(),
            spread_method: krilla::paint::SpreadMethod::Pad,
            stops: stops(st),
            anti_alias: true,
        }
        .into(),
    };
    Fill { paint, opacity: NormalizedF32::ONE, rule: FillRule::NonZero }
}

pub(crate) fn ok(v: f32) -> bool {
    v.is_finite() && v.abs() < 1e6
}

pub(crate) fn rect_path(r: &Rect) -> Option<Path> {
    if !(ok(r.x) && ok(r.y) && ok(r.w) && ok(r.h)) || r.w <= 0.0 || r.h <= 0.0 {
        return None;
    }
    let mut pb = PathBuilder::new();
    pb.push_rect(krilla::geom::Rect::from_xywh(r.x, r.y, r.w, r.h)?);
    pb.finish()
}

/// `r` grown by `g` points on every side and moved by (`dx`, `dy`).
pub(crate) fn grown(r: &Rect, g: f32, dx: f32, dy: f32) -> Rect {
    Rect { x: r.x - g + dx, y: r.y - g + dy, w: r.w + 2.0 * g, h: r.h + 2.0 * g }
}

/// A shape's shadow and glow, behind it: bands of its silhouette (the same approximation of a
/// blur as the raster renderer, see [`wordcraft_doc::effects::bands`]).
pub(crate) fn shape_effects(
    s: &mut Surface,
    (kind, corner): (ShapeKind, Option<u32>),
    rect: &Rect,
    fx: &wordcraft_doc::effects::ShapeEffects,
    (filled, stroked, sw): (bool, bool, f32),
) {
    let sw = if sw.is_finite() { sw.clamp(0.25, 200.0) } else { 0.75 };
    let silhouette = |s: &mut Surface, c: Rgb, a: f32, g: f32, dx: f32, dy: f32| {
        if filled {
            let grow = g + if stroked { sw / 2.0 } else { 0.0 };
            if let Some(p) = shape_path(kind, &grown(rect, grow, dx, dy), corner) {
                s.set_stroke(None);
                s.set_fill(Some(fill(c, a)));
                s.draw_path(&p);
            }
        } else if sw + 2.0 * g > 0.05
            && let Some(p) = shape_path(kind, &grown(rect, 0.0, dx, dy), corner)
        {
            s.set_fill(None);
            s.set_stroke(Some(Stroke { paint: rgb::Color::new(c.0, c.1, c.2).into(), width: sw + 2.0 * g, opacity: norm(a), ..Default::default() }));
            s.draw_path(&p);
        }
    };
    if let Some(sh) = fx.shadow {
        let (dx, dy) = sh.offset();
        for (g, a) in wordcraft_doc::effects::bands(-sh.blur / 2.0, sh.blur / 2.0, sh.opacity()) {
            silhouette(s, sh.color, a, g, dx, dy);
        }
    }
    if let Some(gl) = fx.glow {
        for (g, a) in wordcraft_doc::effects::bands(0.0, gl.size, gl.opacity()) {
            silhouette(s, gl.color, a, g, 0.0, 0.0);
        }
    }
    s.set_fill(None);
    s.set_stroke(None);
}

/// Outline of a basic shape in a rectangle (same geometry as the raster renderer); `corner` is a
/// rounded rectangle's ([`wordcraft_doc::connector::ShapeExtra::corner`]).
pub(crate) fn shape_path(kind: ShapeKind, r: &Rect, corner: Option<u32>) -> Option<Path> {
    let flat_ok = kind.is_open() && r.w.max(r.h) > 0.0 && r.w.min(r.h) >= 0.0;
    if !(ok(r.x) && ok(r.y) && ok(r.w) && ok(r.h)) || !(flat_ok || r.w > 0.0 && r.h > 0.0) {
        return None;
    }
    let (x0, y0, x1, y1) = (r.x, r.y, r.x + r.w, r.y + r.h);
    let (cx, cy) = (x0 + r.w / 2.0, y0 + r.h / 2.0);
    let mut pb = PathBuilder::new();
    let poly = |pb: &mut PathBuilder, pts: &[(f32, f32)]| {
        for (i, (x, y)) in pts.iter().enumerate() {
            if i == 0 {
                pb.move_to(*x, *y);
            } else {
                pb.line_to(*x, *y);
            }
        }
        pb.close();
    };
    const K: f32 = 0.552_284_8;
    let ellipse = |pb: &mut PathBuilder, cx: f32, cy: f32, rx: f32, ry: f32| {
        pb.move_to(cx + rx, cy);
        pb.cubic_to(cx + rx, cy + ry * K, cx + rx * K, cy + ry, cx, cy + ry);
        pb.cubic_to(cx - rx * K, cy + ry, cx - rx, cy + ry * K, cx - rx, cy);
        pb.cubic_to(cx - rx, cy - ry * K, cx - rx * K, cy - ry, cx, cy - ry);
        pb.cubic_to(cx + rx * K, cy - ry, cx + rx, cy - ry * K, cx + rx, cy);
        pb.close();
    };
    match kind {
        // A freeform is drawn from its own paths; without them, its frame.
        ShapeKind::Rectangle | ShapeKind::TextBox | ShapeKind::Freeform => poly(&mut pb, &[(x0, y0), (x1, y0), (x1, y1), (x0, y1)]),
        ShapeKind::RoundedRectangle => {
            let rad = wordcraft_doc::connector::corner_radius(corner, r.w, r.h);
            let k = rad * (1.0 - K);
            pb.move_to(x0 + rad, y0);
            pb.line_to(x1 - rad, y0);
            pb.cubic_to(x1 - k, y0, x1, y0 + k, x1, y0 + rad);
            pb.line_to(x1, y1 - rad);
            pb.cubic_to(x1, y1 - k, x1 - k, y1, x1 - rad, y1);
            pb.line_to(x0 + rad, y1);
            pb.cubic_to(x0 + k, y1, x0, y1 - k, x0, y1 - rad);
            pb.line_to(x0, y0 + rad);
            pb.cubic_to(x0, y0 + k, x0 + k, y0, x0 + rad, y0);
            pb.close();
        }
        ShapeKind::Ellipse => ellipse(&mut pb, cx, cy, r.w / 2.0, r.h / 2.0),
        ShapeKind::FlowchartOr => {
            ellipse(&mut pb, cx, cy, r.w / 2.0, r.h / 2.0);
            pb.move_to(x0, cy);
            pb.line_to(x1, cy);
            pb.move_to(cx, y0);
            pb.line_to(cx, y1);
        }
        ShapeKind::Hexagon
        | ShapeKind::Octagon
        | ShapeKind::Pentagon
        | ShapeKind::Parallelogram
        | ShapeKind::Trapezoid
        | ShapeKind::Plus
        | ShapeKind::Chevron
        | ShapeKind::Decagon => {
            let unit = wordcraft_doc::preset::polygon(kind, r.w, r.h).unwrap_or_default();
            let pts: Vec<(f32, f32)> = unit.iter().map(|(x, y)| (x0 + x * r.w, y0 + y * r.h)).collect();
            poly(&mut pb, &pts)
        }
        ShapeKind::Triangle => poly(&mut pb, &[(cx, y0), (x1, y1), (x0, y1)]),
        ShapeKind::RightTriangle => poly(&mut pb, &[(x0, y0), (x1, y1), (x0, y1)]),
        ShapeKind::Diamond => poly(&mut pb, &[(cx, y0), (x1, cy), (cx, y1), (x0, cy)]),
        // DrawingML's `line`: top-left to bottom-right (flips give the other directions).
        ShapeKind::Line => {
            pb.move_to(x0, y0);
            pb.line_to(x1, y1);
        }
        // Connectors are drawn as paths (`Draw::Path`); this is their frame's diagonal.
        ShapeKind::StraightConnector | ShapeKind::ElbowConnector | ShapeKind::CurvedConnector => {
            pb.move_to(x0, y0);
            pb.line_to(x1, y1);
        }
        ShapeKind::Arrow | ShapeKind::ArrowLeft | ShapeKind::ArrowUp | ShapeKind::ArrowDown => {
            let pts: Vec<(f32, f32)> =
                wordcraft_doc::wrap::arrow_outline(kind, r.w, r.h).iter().map(|&(u, v)| (x0 + u * r.w, y0 + v * r.h)).collect();
            poly(&mut pb, &pts)
        }
        ShapeKind::Star => {
            let pts: Vec<(f32, f32)> = (0..10)
                .map(|i| {
                    let a = std::f32::consts::PI * i as f32 / 5.0 - std::f32::consts::FRAC_PI_2;
                    let rad = if i % 2 == 0 { 1.0 } else { 0.4 };
                    (cx + a.cos() * r.w / 2.0 * rad, cy + a.sin() * r.h / 2.0 * rad)
                })
                .collect();
            poly(&mut pb, &pts);
        }
        ShapeKind::Heart => {
            let (w, h) = (r.w, r.h);
            pb.move_to(cx, y0 + h * 0.3);
            pb.cubic_to(cx, y0, x0, y0, x0, y0 + h * 0.3);
            pb.cubic_to(x0, y0 + h * 0.6, cx - w * 0.1, y0 + h * 0.75, cx, y1);
            pb.cubic_to(cx + w * 0.1, y0 + h * 0.75, x1, y0 + h * 0.6, x1, y0 + h * 0.3);
            pb.cubic_to(x1, y0, cx, y0, cx, y0 + h * 0.3);
            pb.close();
        }
    }
    pb.finish()
}

/// Map the document cap to the PDF backend for both shape and path outlines.
pub(crate) fn pdf_line_cap(cap: wordcraft_doc::connector::LineCap) -> LineCap {
    match cap {
        wordcraft_doc::connector::LineCap::Flat => LineCap::Butt,
        wordcraft_doc::connector::LineCap::Square => LineCap::Square,
        wordcraft_doc::connector::LineCap::Round => LineCap::Round,
    }
}
