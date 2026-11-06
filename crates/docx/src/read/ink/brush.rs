//! Visual channel resolution and swept nib geometry. Filled compound paths are painted once
//! per continuous color/opacity run, avoiding darker overlaps at a translucent stroke's joins.

use super::{
    El, Graphic, GraphicItem, GraphicKind, PathSeg, Rgb, bounded,
    format::{Format, unit},
    number, spend,
    trace::Sample,
};
use wordcraft_doc::para::ShapeKind;

/// A predefined InkML nib shape in its unrotated coordinate system.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(super) enum Tip {
    /// Ellipse (circle when width equals height).
    #[default]
    Ellipse,
    /// Axis-aligned rectangle before applying the OR channel.
    Rectangle,
    /// Circle with tangent edges meeting at a point above it.
    Drop,
}

/// Properties that override sample channels when explicitly specified.
#[derive(Clone, Default)]
pub(super) struct Brush {
    /// Optional constant RGB brush color.
    pub color: Option<Rgb>,
    /// Optional constant horizontal nib diameter in points.
    pub width: Option<f64>,
    /// Optional constant vertical nib diameter in points.
    pub height: Option<f64>,
    /// Constant opacity; absent means read the A channel if present.
    pub alpha: Option<f64>,
    /// Predefined nib shape.
    pub tip: Tip,
    /// Ignore force even when a calibrated F channel is available.
    pub ignore_pressure: bool,
    /// Smooth sample positions with an application-defined interpolating curve.
    pub curve: bool,
    /// Preserve a no-operation trace without painting a fallback picture.
    pub invisible: bool,
}

impl Brush {
    /// Apply explicit properties to an inherited brush; unknown visual semantics reject preview.
    pub fn properties(&mut self, el: &El, work: &mut usize) -> Option<()> {
        for p in el.els() {
            spend(work, p.attrs.len().saturating_mul(4).max(1))?;
            if matches!(p.name.as_str(), "ink:annotation" | "ink:annotationXML") {
                continue;
            }
            if p.name != "ink:brushProperty" {
                return None;
            }
            let value = p.attr("value")?;
            match p.attr("name")? {
                "color" => self.color = Some(Rgb::parse(value)?),
                "width" | "height" => {
                    let size = number(value)? * unit(p.attr("units"))?;
                    if !(0.001..=100_000.0).contains(&size) {
                        return None;
                    }
                    if p.attr("name") == Some("width") {
                        self.width = Some(size);
                    } else {
                        self.height = Some(size);
                    }
                }
                "transparency" => {
                    let v = number(value)?;
                    // InkML leaves the integer range application-defined: use the common octet
                    // convention; explicit percent units use the specification's 0..1 fractions.
                    let a = if p.attr("units") == Some("%") {
                        v
                    } else if p.attr("units").is_none() {
                        v / 255.0
                    } else {
                        return None;
                    };
                    if !(0.0..=1.0).contains(&a) {
                        return None;
                    }
                    self.alpha = Some(1.0 - a);
                }
                "tip" => {
                    self.tip = match value {
                        "ellipse" => Tip::Ellipse,
                        "rectangle" => Tip::Rectangle,
                        "drop" => Tip::Drop,
                        _ => return None,
                    }
                }
                "rasterOp" => {
                    self.invisible = match value {
                        "copyPen" => false,
                        "noOperation" => true,
                        _ => return None,
                    }
                }
                "ignorePressure" => self.ignore_pressure = boolean(value)?,
                "fitToCurve" => self.curve = boolean(value)?,
                // Anti-aliasing is a renderer policy, not a change to the stored stroke geometry.
                "antiAliased" => {
                    boolean(value)?;
                }
                _ => return None,
            }
        }
        Some(())
    }

    /// Resolve a sample's visual channels, respecting constant brush-property precedence.
    pub fn point(&self, format: &Format, values: &Sample, xy: [f64; 2]) -> Option<Point> {
        let channel =
            |name: &str, fallback| -> Option<f64> { if format.index(name).is_some() { format.value(values, name) } else { Some(fallback) } };
        let mut width = match self.width {
            Some(v) => v,
            None if format.index("BW").is_some() => format.value(values, "BW")?,
            None => channel("W", 1.0)?,
        };
        let mut height = match self.height {
            Some(v) => v,
            None => channel("BH", width)?,
        };
        if !self.ignore_pressure && format.index("F").is_some() {
            // Pressure response is not specified by InkML; use a documented linear response
            // only when a range/percentage allows unambiguous normalization.
            if let Some(force) = format.fraction(values, "F") {
                width *= force;
                height *= force;
            }
        }
        if !width.is_finite() || !height.is_finite() || !(0.0..=100_000.0).contains(&width) || !(0.0..=100_000.0).contains(&height) {
            return None;
        }
        let color = match self.color {
            Some(c) => c,
            None if format.index("C").is_some() => {
                let c = format.value(values, "C")?;
                if !(0.0..=16_777_215.0).contains(&c) || c.fract() != 0.0 {
                    return None;
                }
                let c = c as u32;
                Rgb((c >> 16) as u8, (c >> 8) as u8, c as u8)
            }
            None if ["CR", "CG", "CB"].iter().any(|n| format.index(n).is_some()) => {
                let component = |name| -> Option<u8> { Some((format.fraction(values, name)? * 255.0).round() as u8) };
                Rgb(component("CR")?, component("CG")?, component("CB")?)
            }
            None if ["CC", "CM", "CY", "CK"].iter().any(|n| format.index(n).is_some()) => {
                let k = format.fraction(values, "CK")?;
                let component = |name| -> Option<u8> { Some(((1.0 - format.fraction(values, name)?) * (1.0 - k) * 255.0).round() as u8) };
                Rgb(component("CC")?, component("CM")?, component("CY")?)
            }
            None => Rgb::BLACK,
        };
        let alpha = if self.invisible {
            0.0
        } else {
            match self.alpha {
                Some(v) => v,
                None if format.index("A").is_some() => 1.0 - format.fraction(values, "A")?,
                None => 1.0,
            }
        };
        let angle = channel("OR", 0.0)?;
        Some(Point { xy, width, height, angle, color, alpha, tip: self.tip })
    }
}

/// Parse the XML boolean spelling used by standard brush flags.
fn boolean(value: &str) -> Option<bool> {
    match value {
        "true" | "1" => Some(true),
        "false" | "0" => Some(false),
        _ => None,
    }
}

/// One pen-down sample after context/mapping/brush resolution.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct Point {
    /// Physical canvas coordinates.
    pub xy: [f64; 2],
    /// Horizontal nib diameter.
    pub width: f64,
    /// Vertical nib diameter.
    pub height: f64,
    /// Counter-clockwise nib rotation in radians.
    pub angle: f64,
    /// Resolved RGB color.
    pub color: Rgb,
    /// Resolved opacity.
    pub alpha: f64,
    /// Resolved nib shape.
    pub tip: Tip,
}

/// A pen-down run, possibly assembled from several continuation fragments.
pub(super) struct Stroke {
    /// Ordered samples, never including a pen-up bridge.
    pub points: Vec<Point>,
    /// Whether the centerline requests curve fitting.
    pub curve: bool,
}

/// A disposable f64 polygon or simple centerline before subtracting the drawing origin.
struct Geometry {
    /// Compound filled polygon coordinates, or a simple centerline for the legacy round-stroke path.
    polygons: Vec<Vec<[f64; 2]>>,
    /// Paint color.
    color: Rgb,
    /// Paint opacity.
    alpha: f64,
    /// A circular constant-width pen may use the existing round-cap native ink renderer.
    width: Option<f64>,
}

/// Generate bounded geometry and reduce large origins before f32 conversion.
pub(super) fn drawing(strokes: Vec<Stroke>, work: &mut usize) -> Option<Graphic> {
    let mut geometries = Vec::new();
    for stroke in strokes {
        let first = *stroke.points.first()?;
        let simple = !stroke.curve
            && first.tip == Tip::Ellipse
            && first.width == first.height
            && first.alpha == 1.0
            && first.width > 0.0
            && stroke.points.iter().all(|p| p.width == first.width && p.height == first.height && p.color == first.color && p.alpha == first.alpha);
        if simple {
            spend(work, stroke.points.len())?;
            geometries.push(Geometry {
                polygons: vec![stroke.points.into_iter().map(|p| p.xy).collect()],
                color: first.color,
                alpha: 1.0,
                width: Some(first.width),
            });
            continue;
        }
        let points = if stroke.curve { smooth(&stroke.points, work)? } else { stroke.points };
        let first_geometry = geometries.len();
        let mut previous: Option<Vec<[f64; 2]>> = None;
        for p in points {
            spend(work, 128)?;
            let nib = nib(p)?;
            let polygon = match previous.as_ref() {
                Some(prior) => hull(prior.iter().chain(&nib).copied().collect()),
                None => nib.clone(),
            };
            previous = Some(nib);
            let same_stroke = geometries.len() > first_geometry;
            if let Some(g) = geometries.last_mut().filter(|g| same_stroke && g.width.is_none() && g.color == p.color && g.alpha == p.alpha) {
                g.polygons.push(polygon);
            } else {
                geometries.push(Geometry { polygons: vec![polygon], color: p.color, alpha: p.alpha, width: None });
            }
        }
    }
    let (mut left, mut top, mut right, mut bottom) = (f64::INFINITY, f64::INFINITY, f64::NEG_INFINITY, f64::NEG_INFINITY);
    for g in &geometries {
        let r = g.width.unwrap_or(0.0) / 2.0;
        for [x, y] in g.polygons.iter().flatten() {
            left = left.min(x - r);
            top = top.min(y - r);
            right = right.max(x + r);
            bottom = bottom.max(y + r);
        }
    }
    let (w, h) = ((right - left).max(0.001), (bottom - top).max(0.001));
    if !w.is_finite() || !h.is_finite() {
        return None;
    }
    let scale = (f64::from(crate::units::MAX_LEN_PT) / w.max(h)).min(1.0);
    let point = |[x, y]: [f64; 2]| (((x - left) * scale) as f32, ((y - top) * scale) as f32);
    let mut items = Vec::new();
    for g in geometries {
        let mut segs = Vec::new();
        for polygon in &g.polygons {
            spend(work, polygon.len() + 1)?;
            for (i, p) in polygon.iter().enumerate() {
                let (x, y) = point(*p);
                segs.push(if i == 0 { PathSeg::Move(x, y) } else { PathSeg::Line(x, y) });
            }
            if g.width.is_none() {
                segs.push(PathSeg::Close);
            }
        }
        if let Some(width) = g.width {
            let width = (width * scale) as f32;
            if let [PathSeg::Move(x, y)] = segs.as_slice() {
                items.push(GraphicItem::Shape {
                    rect: [x - width / 2.0, y - width / 2.0, width, width],
                    kind: ShapeKind::Ellipse,
                    fill: Some(g.color),
                    stroke: None,
                    stroke_width: 0.0,
                });
            } else {
                items.push(GraphicItem::Path { segs, fill: None, stroke: Some(g.color), stroke_width: width });
            }
        } else {
            items.push(GraphicItem::InkOutline { segs, color: g.color, alpha: g.alpha as f32 });
        }
    }
    (!items.is_empty()).then_some(Graphic { kind: GraphicKind::Ink, items, w: (w * scale) as f32, h: (h * scale) as f32, ..Default::default() })
}

/// Sample a convex nib with a fixed vertex bound; rotation follows InkML's counter-clockwise OR.
fn nib(p: Point) -> Option<Vec<[f64; 2]>> {
    let (rx, ry) = (p.width / 2.0, p.height / 2.0);
    let mut points = match p.tip {
        Tip::Rectangle => vec![[-rx, -ry], [rx, -ry], [rx, ry], [-rx, ry]],
        Tip::Ellipse | Tip::Drop => (0..24)
            .map(|i| {
                let a = std::f64::consts::TAU * f64::from(i) / 24.0;
                [rx * a.cos(), if p.tip == Tip::Drop { rx * a.sin() } else { ry * a.sin() }]
            })
            .collect(),
    };
    if p.tip == Tip::Drop {
        if p.height < p.width {
            return None;
        }
        points.push([0.0, rx - p.height]);
        points = hull(points);
    }
    let (s, c) = p.angle.sin_cos();
    for point in &mut points {
        let [x, y] = *point;
        *point = [p.xy[0] + x * c + y * s, p.xy[1] - x * s + y * c];
        if !bounded(point[0]) || !bounded(point[1]) {
            return None;
        }
    }
    Some(points)
}

/// Monotone-chain convex hull over at most two 25-vertex nibs, consistently wound for nonzero fill.
fn hull(mut points: Vec<[f64; 2]>) -> Vec<[f64; 2]> {
    points.sort_by(|a, b| a[0].total_cmp(&b[0]).then(a[1].total_cmp(&b[1])));
    points.dedup();
    if points.len() < 3 {
        return points;
    }
    let cross = |a: [f64; 2], b: [f64; 2], c: [f64; 2]| (b[0] - a[0]) * (c[1] - a[1]) - (b[1] - a[1]) * (c[0] - a[0]);
    let mut out: Vec<[f64; 2]> = Vec::new();
    for p in &points {
        while let [.., a, b] = out.as_slice() {
            if cross(*a, *b, *p) > 0.0 {
                break;
            }
            out.pop();
        }
        out.push(*p);
    }
    let lower = out.len();
    for p in points.iter().rev().skip(1) {
        while out.len() > lower {
            let [.., a, b] = out.as_slice() else {
                break;
            };
            if cross(*a, *b, *p) > 0.0 {
                break;
            }
            out.pop();
        }
        out.push(*p);
    }
    out.pop();
    out
}

/// Bounded Catmull–Rom interpolation; InkML does not prescribe a particular curve fitter.
fn smooth(points: &[Point], work: &mut usize) -> Option<Vec<Point>> {
    let mut out = Vec::new();
    for (i, p1) in points.iter().enumerate() {
        out.push(*p1);
        let Some(p2) = points.get(i + 1) else {
            break;
        };
        let p0 = i.checked_sub(1).and_then(|j| points.get(j)).unwrap_or(p1);
        let p3 = points.get(i + 2).unwrap_or(p2);
        spend(work, 7 * 8)?;
        for j in 1..8 {
            let t = f64::from(j) / 8.0;
            let mut p = *p1;
            for (axis, v) in p.xy.iter_mut().enumerate() {
                let (a, b, c, d) = (p0.xy.get(axis)?, p1.xy.get(axis)?, p2.xy.get(axis)?, p3.xy.get(axis)?);
                *v = 0.5 * ((2.0 * b) + (-a + c) * t + (2.0 * a - 5.0 * b + 4.0 * c - d) * t * t + (-a + 3.0 * b - 3.0 * c + d) * t * t * t);
                if !bounded(*v) {
                    return None;
                }
            }
            p.width += (p2.width - p1.width) * t;
            p.height += (p2.height - p1.height) * t;
            p.angle += (p2.angle - p1.angle) * t;
            out.push(p);
        }
    }
    Some(out)
}
