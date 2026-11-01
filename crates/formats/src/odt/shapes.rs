//! Drawing shapes (ODF 1.3 §10.3): `draw:rect`, `draw:ellipse`, `draw:line` and
//! `draw:custom-shape`, whose `draw:enhanced-geometry` (§10.6.2) names its predefined geometry
//! in `draw:type`. WordCraft's shape kinds both ways; LibreOffice rebuilds a predefined
//! geometry from its type alone, so only the type is written.

use super::xml::{Attrs, get, length};
use wordcraft_doc::para::ShapeKind;

/// Largest size or offset read, points (22 inches).
const MAX_LEN: f32 = 1584.0;

/// Whether `q` is a drawing shape element read as a shape.
pub(super) fn is_shape(q: &str) -> bool {
    matches!(q, "draw:rect" | "draw:ellipse" | "draw:line" | "draw:custom-shape")
}

/// The kind of shape element `name` (local name); a custom shape's geometry says more.
pub(super) fn element_kind(name: &str) -> ShapeKind {
    match name {
        "ellipse" => ShapeKind::Ellipse,
        "line" => ShapeKind::Line,
        _ => ShapeKind::Rectangle,
    }
}

/// The kind of a custom shape of `draw:type` `t`: LibreOffice's own type names, and the
/// DrawingML preset names it writes as `ooxml-…` for shapes from Word documents. A geometry
/// WordCraft has no shape for is a rectangle.
pub(super) fn kind_of(t: &str) -> ShapeKind {
    let t = t.trim();
    match t.strip_prefix("ooxml-").unwrap_or(t) {
        "round-rectangle" | "roundRect" => ShapeKind::RoundedRectangle,
        "ellipse" | "flowchart-connector" | "flowChartConnector" => ShapeKind::Ellipse,
        "flowchart-or" | "flowChartOr" => ShapeKind::FlowchartOr,
        "isosceles-triangle" | "triangle" => ShapeKind::Triangle,
        "right-triangle" | "rtTriangle" => ShapeKind::RightTriangle,
        "diamond" | "flowchart-decision" | "flowChartDecision" => ShapeKind::Diamond,
        "right-arrow" | "rightArrow" => ShapeKind::Arrow,
        "left-arrow" | "leftArrow" => ShapeKind::ArrowLeft,
        "up-arrow" | "upArrow" => ShapeKind::ArrowUp,
        "down-arrow" | "downArrow" => ShapeKind::ArrowDown,
        "star5" => ShapeKind::Star,
        "heart" => ShapeKind::Heart,
        "line" | "straightConnector1" => ShapeKind::Line,
        _ => ShapeKind::Rectangle,
    }
}

/// How shape `kind` is written: its element, and a custom shape's `draw:type`.
pub(super) fn element_of(kind: ShapeKind) -> (&'static str, Option<&'static str>) {
    let custom = |t| ("draw:custom-shape", Some(t));
    match kind {
        ShapeKind::Ellipse => ("draw:ellipse", None),
        ShapeKind::Line | ShapeKind::StraightConnector | ShapeKind::ElbowConnector | ShapeKind::CurvedConnector => ("draw:line", None),
        ShapeKind::RoundedRectangle => custom("round-rectangle"),
        ShapeKind::FlowchartOr => custom("flowchart-or"),
        ShapeKind::Triangle => custom("isosceles-triangle"),
        ShapeKind::RightTriangle => custom("right-triangle"),
        ShapeKind::Diamond => custom("diamond"),
        ShapeKind::Arrow => custom("right-arrow"),
        ShapeKind::ArrowLeft => custom("left-arrow"),
        ShapeKind::ArrowUp => custom("up-arrow"),
        ShapeKind::ArrowDown => custom("down-arrow"),
        ShapeKind::Star => custom("star5"),
        ShapeKind::Heart => custom("heart"),
        ShapeKind::Rectangle | ShapeKind::TextBox | ShapeKind::Freeform => ("draw:rect", None),
    }
}

/// A shape's size and offset (`svg:width`, `svg:height`, `svg:x`, `svg:y`), points, and which
/// way it is drawn (from right to left, from bottom to top). A line's come from its two ends
/// (`svg:x1`…): its box spans them, and it starts at the first end, so a line drawn leftwards
/// or upwards is flipped that way (its arrowheads stay at its ends).
pub(super) fn geometry(kind: ShapeKind, a: &Attrs) -> (f32, f32, Option<f32>, Option<f32>, (bool, bool)) {
    let len = |k: &str| get(a, k).and_then(length).filter(|v| v.is_finite()).map(|v| v.clamp(-MAX_LEN, MAX_LEN));
    if kind == ShapeKind::Line {
        let [x1, y1, x2, y2] = ["svg:x1", "svg:y1", "svg:x2", "svg:y2"].map(|k| len(k).unwrap_or(0.0));
        let size = |d: f32| d.abs().min(MAX_LEN);
        return (size(x2 - x1), size(y2 - y1), Some(x1.min(x2)), Some(y1.min(y2)), (x2 < x1, y2 < y1));
    }
    let size = |k: &str| len(k).map_or(72.0, |v| v.clamp(1.0, MAX_LEN));
    (size("svg:width"), size("svg:height"), len("svg:x"), len("svg:y"), (false, false))
}
