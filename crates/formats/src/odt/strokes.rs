//! The named drawing definitions graphic styles use (ODF 1.3 §16.39–16.41): line dashes
//! (`draw:stroke-dash`), gradients (`draw:gradient`) and arrowheads (`draw:marker`), read into
//! WordCraft's dash patterns and gradient fills and written from them.

use super::writer::pt;
use super::xml::{Attrs, get, length};
use crate::model::parse_color;
use std::sync::Arc;
use wordcraft_doc::fill::{Fill, Gradient, GradientKind, PathShape};
use wordcraft_doc::line_dash::{DASH_PERCENT, LineDash, MAX_DASH_STOPS};
use wordcraft_doc::props::Rgb;
use wordcraft_doc::wordart::GradStop;

/// The arrowhead WordCraft writes (`draw:marker`, §16.39): a plain triangle pointing along the
/// line, its tip at the line's end.
pub(super) const ARROW: &str = "WordCraft_20_Arrow";
pub(super) const ARROW_DEF: &str =
    "<draw:marker draw:name=\"WordCraft_20_Arrow\" draw:display-name=\"WordCraft Arrow\" svg:viewBox=\"0 0 20 20\" svg:d=\"M10 0L20 20H0z\"/>";

/// A dash or space length: a fraction of the line width, or an absolute length in points.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Len {
    Width(f32),
    Points(f32),
}

impl Len {
    fn read(v: Option<&str>) -> Option<Len> {
        let v = v?.trim();
        match v.strip_suffix('%') {
            Some(p) => p.trim().parse::<f32>().ok().filter(|p| p.is_finite()).map(|p| Len::Width((p / 100.0).clamp(0.0, 100.0))),
            None => length(v).filter(|l| l.is_finite()).map(|l| Len::Points(l.clamp(0.0, 1584.0))),
        }
    }

    /// In 100000ths of a line `width` points wide.
    fn of(self, width: f32) -> u32 {
        let ratio = match self {
            Len::Width(r) => r,
            Len::Points(p) => {
                let w = if width.is_finite() && width > 0.0 { width } else { 0.75 };
                p / w
            }
        };
        (ratio.clamp(0.0, 1000.0) * DASH_PERCENT as f32).round() as u32
    }
}

/// A `draw:stroke-dash`: `dots1` dashes of one length then `dots2` of another, each followed by
/// the same space.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct Dash {
    dots1: u32,
    len1: Len,
    dots2: u32,
    len2: Len,
    space: Len,
}

impl Dash {
    pub(super) fn read(a: &Attrs) -> Dash {
        let count = |k: &str| get(a, k).and_then(|v| v.trim().parse::<u32>().ok()).unwrap_or(0).min(MAX_DASH_STOPS as u32);
        // A dash without a length is a dot as long as the line is wide.
        let len = |k: &str| Len::read(get(a, k)).unwrap_or(Len::Width(1.0));
        Dash {
            dots1: count("draw:dots1"),
            len1: len("draw:dots1-length"),
            dots2: count("draw:dots2"),
            len2: len("draw:dots2-length"),
            space: len("draw:distance"),
        }
    }

    /// The pattern for a line `width` points wide: one of WordCraft's named dashes when it is one.
    pub(super) fn pattern(&self, width: f32) -> LineDash {
        let space = self.space.of(width);
        let pairs: Vec<(u32, u32)> = std::iter::repeat_n((self.len1.of(width), space), self.dots1 as usize)
            .chain(std::iter::repeat_n((self.len2.of(width), space), self.dots2 as usize))
            .take(MAX_DASH_STOPS)
            .collect();
        if pairs.is_empty() {
            return LineDash::Solid;
        }
        PRESETS.iter().find(|p| ratios(p) == pairs).cloned().unwrap_or(LineDash::Custom(pairs))
    }
}

/// WordCraft's named dashes.
const PRESETS: [LineDash; 10] = [
    LineDash::Dot,
    LineDash::Dash,
    LineDash::LongDash,
    LineDash::DashDot,
    LineDash::LongDashDot,
    LineDash::LongDashDotDot,
    LineDash::SystemDash,
    LineDash::SystemDot,
    LineDash::SystemDashDot,
    LineDash::SystemDashDotDot,
];

/// A dash's (dash, space) pairs in 100000ths of the line width.
fn ratios(d: &LineDash) -> Vec<(u32, u32)> {
    if let LineDash::Custom(pairs) = d {
        return pairs.iter().take(MAX_DASH_STOPS).copied().collect();
    }
    let p: Vec<u32> = d.pattern(1.0).iter().map(|v| (v * DASH_PERCENT as f32).round() as u32).collect();
    p.chunks(2).map(|c| (c.first().copied().unwrap_or(0), c.get(1).copied().unwrap_or(0))).collect()
}

/// The `draw:stroke-dash` named `name` for `dash`, or `None` for a solid line. ODF's dash has
/// at most two kinds of dash and one space: a pattern with more is written as its first two.
pub(super) fn dash_def(name: &str, dash: &LineDash) -> Option<String> {
    let pairs = ratios(dash);
    let (&(l1, space), rest) = pairs.split_first()?;
    let dots1 = 1 + rest.iter().take_while(|p| **p == (l1, space)).count();
    let second = rest.get(dots1 - 1).map(|p| p.0);
    let dots2 = second.map_or(0, |l2| rest.iter().skip(dots1 - 1).take_while(|p| p.0 == l2).count());
    let pc = |v: u32| format!("{}%", v as f32 * 100.0 / DASH_PERCENT as f32);
    let mut s = format!("<draw:stroke-dash draw:name=\"{name}\" draw:style=\"rect\" draw:dots1=\"{dots1}\" draw:dots1-length=\"{}\"", pc(l1));
    if let Some(l2) = second {
        s.push_str(&format!(" draw:dots2=\"{dots2}\" draw:dots2-length=\"{}\"", pc(l2)));
    }
    s.push_str(&format!(" draw:distance=\"{}\"/>", pc(space)));
    Some(s)
}

/// An angle (§18.3.1): degrees, radians or grads by its unit, tenths of a degree without one
/// (as ODF 1.2 wrote a gradient's).
/// Whether a part of ODF `version` (its `office:version`; none: before 1.2) has its gradients'
/// and hatches' unitless angles in tenths of a degree.
///
/// **A deviation from the standard, out of necessity.** ODF's angle (§18.3.1, since ODF 1.2)
/// is in degrees when it has no unit, so `draw:angle="300"` (§19.112) and `draw:rotation`
/// (§19.209) mean 300°. But LibreOffice and OpenOffice wrote these angles in tenths of a degree
/// in their files before ODF 1.3 (`300` for 30°), and LibreOffice still reads such files so
/// (observed: the same gradient turns 30° in an ODF 1.2 file and 300° in an ODF 1.3 one). Read
/// by the standard, the gradients of every older LibreOffice document would turn the wrong
/// way. So for files older than 1.3 WordCraft reads them as written; from 1.3 on, as the
/// standard says. WordCraft writes them with a unit (`30deg`), which both read the same.
pub(super) fn legacy_angles(version: Option<&str>) -> bool {
    version.is_none_or(|v| v.trim() < "1.3")
}

fn angle(v: &str, legacy: bool) -> Option<f32> {
    let v = v.trim();
    let (n, scale) = if let Some(n) = v.strip_suffix("deg") {
        (n, 1.0)
    } else if let Some(n) = v.strip_suffix("grad") {
        (n, 0.9)
    } else if let Some(n) = v.strip_suffix("rad") {
        (n, 180.0 / std::f32::consts::PI)
    } else if legacy {
        // A deviation from the standard, out of necessity: see `legacy_angles`.
        (v, 0.1)
    } else {
        // §18.3.1: without a unit, degrees.
        (v, 1.0)
    };
    n.trim().parse::<f32>().ok().map(|a| a * scale).filter(|a| a.is_finite()).map(|a| a.rem_euclid(360.0))
}

/// A percentage (`50%`), 0–100.
fn percent(v: Option<&str>) -> Option<f32> {
    v?.trim().strip_suffix('%')?.trim().parse::<f32>().ok().filter(|p| p.is_finite()).map(|p| p.clamp(0.0, 100.0))
}

/// A `draw:gradient` (§16.40) as a fill. Linear and axial gradients run along their angle (ODF's
/// 0° runs top to bottom, turning anticlockwise; WordCraft's 0° left to right, clockwise);
/// radial, ellipsoid, square and rectangular ones run from their end colour in the middle to
/// their start colour at the edge. The border is where the start colour stays plain; intensity
/// darkens a colour.
pub(super) fn gradient(a: &Attrs, legacy: bool) -> Option<Fill> {
    let color = |k: &str, intensity: &str, default: Rgb| {
        let c = get(a, k).and_then(parse_color).unwrap_or(default);
        let i = percent(get(a, intensity)).unwrap_or(100.0) / 100.0;
        let dim = |v: u8| (f32::from(v) * i).round().clamp(0.0, 255.0) as u8;
        Rgb(dim(c.0), dim(c.1), dim(c.2))
    };
    let start = color("draw:start-color", "draw:start-intensity", Rgb::BLACK);
    let end = color("draw:end-color", "draw:end-intensity", Rgb::WHITE);
    let border = percent(get(a, "draw:border")).unwrap_or(0.0);
    let stop = |pos: f32, color: Rgb| GradStop { pos, color, transparency: 0.0 };
    let along = (90.0 - get(a, "draw:angle").and_then(|v| angle(v, legacy)).unwrap_or(0.0)).rem_euclid(360.0);
    let centre = || {
        let (cx, cy) = (percent(get(a, "draw:cx")).unwrap_or(50.0), percent(get(a, "draw:cy")).unwrap_or(50.0));
        [cx, cy, 100.0 - cx, 100.0 - cy]
    };
    let (kind, stops) = match get(a, "draw:style").map(str::trim).unwrap_or("linear") {
        "axial" => (GradientKind::Linear { angle: along, scaled: false }, vec![stop(0.0, start), stop(50.0, end), stop(100.0, start)]),
        style @ ("radial" | "ellipsoid" | "square" | "rectangular") => {
            let shape = if matches!(style, "square" | "rectangular") { PathShape::Rect } else { PathShape::Circle };
            let mut stops = vec![stop(0.0, end)];
            if border > 0.0 {
                stops.push(stop(100.0 - border, start));
            }
            stops.push(stop(100.0, start));
            (GradientKind::Path { shape, focus: centre() }, stops)
        }
        _ => {
            let mut stops = vec![stop(0.0, start)];
            if border > 0.0 {
                stops.push(stop(border, start));
            }
            stops.push(stop(100.0, end));
            (GradientKind::Linear { angle: along, scaled: false }, stops)
        }
    };
    Fill::Gradient(Arc::new(Gradient { stops, kind, tile: None, rot_with_shape: true, flip: Default::default() })).sanitized()
}

/// The `draw:gradient` named `name` for gradient fill `g` (the reverse of [`gradient`]): a
/// gradient running out from its middle and back is axial; more colours than ODF's two keep
/// the first and last.
pub(super) fn gradient_def(name: &str, g: &Gradient) -> Option<String> {
    let hex = |c: Rgb| format!("#{}", c.hex().to_ascii_lowercase());
    let (first, last) = (g.stops.first()?, g.stops.last()?);
    let body = match g.kind {
        GradientKind::Linear { angle, .. } => {
            let odf = (90.0 - if angle.is_finite() { angle } else { 0.0 }).rem_euclid(360.0);
            let axial = g.stops.len() == 3 && first.color == last.color && g.stops.get(1).is_some_and(|m| (m.pos - 50.0).abs() < 0.5);
            let (style, end) = match g.stops.get(1) {
                Some(mid) if axial => ("axial", mid.color),
                _ => ("linear", last.color),
            };
            format!(
                "draw:style=\"{style}\" draw:start-color=\"{}\" draw:end-color=\"{}\" draw:angle=\"{}deg\"",
                hex(first.color),
                hex(end),
                odf.round()
            )
        }
        GradientKind::Path { shape, focus: [l, t, r, b] } => {
            let style = if shape == PathShape::Rect { "rectangular" } else { "radial" };
            let mid = |a: f32, b: f32| if a.is_finite() && b.is_finite() { ((a + 100.0 - b) / 2.0).clamp(0.0, 100.0) } else { 50.0 };
            format!(
                "draw:style=\"{style}\" draw:cx=\"{}%\" draw:cy=\"{}%\" draw:start-color=\"{}\" draw:end-color=\"{}\"",
                mid(l, r),
                mid(t, b),
                hex(last.color),
                hex(first.color)
            )
        }
    };
    Some(format!("<draw:gradient draw:name=\"{name}\" {body} draw:start-intensity=\"100%\" draw:end-intensity=\"100%\" draw:border=\"0%\"/>"))
}

/// The width an arrowhead is drawn at on a line `width` points wide: three times as wide, and
/// at least 5 points.
pub(super) fn arrow_width(width: f32) -> String {
    pt(if width.is_finite() { (width * 3.0).clamp(5.0, 36.0) } else { 5.0 })
}

/// Most named dashes and gradients kept (hostile files).
const MAX_DEFS: usize = 10_000;

/// The named dashes and gradients a document's styles define.
#[derive(Default)]
pub(super) struct Defs {
    dashes: std::collections::HashMap<String, Dash>,
    gradients: std::collections::HashMap<String, Fill>,
    /// The part read is older than ODF 1.3: unitless angles are tenths of a degree (see
    /// [`legacy_angles`]).
    pub(super) legacy_angles: bool,
}

impl Defs {
    /// Element `q` with attributes `a`: a dash or gradient definition, or a part's root, whose
    /// version says how unitless gradient angles are read (see [`legacy_angles`]).
    pub(super) fn element(&mut self, q: &str, a: &Attrs) {
        match q {
            "office:document-styles" | "office:document-content" | "office:document" => self.legacy_angles = legacy_angles(get(a, "office:version")),
            "draw:stroke-dash" => self.read("stroke-dash", a),
            "draw:gradient" => self.read("gradient", a),
            _ => {}
        }
    }

    /// Element `name` (local) with attributes `a`, when it is a definition.
    pub(super) fn read(&mut self, name: &str, a: &Attrs) {
        let Some(id) = get(a, "draw:name").map(String::from) else { return };
        if self.dashes.len() + self.gradients.len() >= MAX_DEFS {
            return;
        }
        match name {
            "stroke-dash" => {
                self.dashes.insert(id, Dash::read(a));
            }
            "gradient" => {
                if let Some(g) = gradient(a, self.legacy_angles) {
                    self.gradients.insert(id, g);
                }
            }
            _ => {}
        }
    }

    pub(super) fn dash(&self, name: &str) -> Option<&Dash> {
        self.dashes.get(name)
    }

    pub(super) fn gradient(&self, name: &str) -> Option<&Fill> {
        self.gradients.get(name)
    }
}

/// The named definitions a writer's graphic styles use, for `styles.xml`.
#[derive(Default)]
pub(super) struct DrawDefs {
    /// Each definition's name by its content (one name for equal definitions).
    names: std::collections::BTreeMap<String, String>,
    /// Name → definition.
    pub(super) defs: std::collections::BTreeMap<String, String>,
}

impl DrawDefs {
    /// The name of the definition `make` writes for `kind` (with this writer's `prefix`).
    fn named(&mut self, kind: &str, prefix: &str, make: impl Fn(&str) -> Option<String>) -> Option<String> {
        let key = make("")?;
        if let Some(n) = self.names.get(&key) {
            return Some(n.clone());
        }
        let name = format!("WordCraft_20_{kind}_20_{prefix}{}", self.names.len() + 1);
        let def = make(&name)?;
        self.names.insert(key, name.clone());
        self.defs.insert(name.clone(), def);
        Some(name)
    }

    /// The `draw:stroke-dash` name for `dash`; `None` for a solid line.
    pub(super) fn dash(&mut self, dash: &LineDash, prefix: &str) -> Option<String> {
        if dash.is_solid() {
            return None;
        }
        self.named("Dash", prefix, |n| dash_def(n, dash))
    }

    /// The `draw:gradient` name for `g`.
    pub(super) fn gradient(&mut self, g: &Gradient, prefix: &str) -> Option<String> {
        self.named("Gradient", prefix, |n| gradient_def(n, g))
    }

    /// The arrowhead's `draw:marker` name.
    pub(super) fn arrow(&mut self) -> &'static str {
        self.defs.entry(ARROW.to_string()).or_insert_with(|| ARROW_DEF.to_string());
        ARROW
    }
}
