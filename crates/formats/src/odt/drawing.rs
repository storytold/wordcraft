//! Drawing shapes and anchored frames (ODF 1.3 §10.3, §10.4): how a graphic style and the
//! element's own attributes say an object looks and where it floats.
use wordcraft_doc::para::{Anchor, Float, FloatAlign, ShapeKind, Wrap, WrapText};
use wordcraft_doc::props::VAlign;

use super::*;

/// Key of the document's default graphic style (`style:default-style style:family="graphic"`),
/// which drawing shapes fall back to. No style name contains a NUL.
pub(super) const DEFAULT_GRAPHIC: &str = "\0graphic";

/// The `style:graphic-properties` attributes read.
const GRAPHIC_KEYS: &[&str] = &[
    "text:anchor-type",
    "draw:fill",
    "draw:fill-color",
    "fo:background-color",
    "draw:stroke",
    "svg:stroke-color",
    "svg:stroke-width",
    "fo:border",
    "fo:padding",
    "fo:padding-left",
    "fo:padding-top",
    "fo:padding-right",
    "fo:padding-bottom",
    "fo:margin-left",
    "fo:margin-right",
    "fo:margin-top",
    "fo:margin-bottom",
    "style:wrap",
    "style:run-through",
    "style:horizontal-pos",
    "style:horizontal-rel",
    "style:vertical-pos",
    "style:vertical-rel",
    "draw:textarea-vertical-align",
];

/// The graphic properties of `a` worth keeping on a style.
pub(super) fn graphic_props(a: &Attrs) -> Attrs {
    a.iter().filter(|(k, _)| GRAPHIC_KEYS.contains(&k.as_str())).cloned().collect()
}

/// Fill, outline and text placement of a shape or text-box frame.
#[derive(Clone, Debug, Default)]
pub(super) struct Look {
    pub fill: Option<Rgb>,
    pub stroke: Option<Rgb>,
    /// Points.
    pub stroke_width: f32,
    /// Internal margins (left, top, right, bottom), EMU.
    pub inset: Option<[u32; 4]>,
    pub valign: VAlign,
}

/// A length in points, clamped to ±22" (the largest page side) so hostile values can't reach
/// layout.
fn clamp_len(v: f32) -> f32 {
    v.clamp(-1584.0, 1584.0)
}

/// `fo:border`'s colour and width (`0.06pt solid #000000`); `None` for no border.
fn border(v: &str) -> Option<(Rgb, f32)> {
    let mut color = None;
    let mut width = None;
    for t in v.split_whitespace() {
        if matches!(t, "none" | "hidden") {
            return None;
        }
        if let Some(c) = model::parse_color(t) {
            color = Some(c);
        } else if let Some(w) = length(t) {
            width = Some(w.clamp(0.0, 1584.0));
        }
    }
    Some((color.unwrap_or(Rgb(0, 0, 0)), width.unwrap_or(0.75)))
}

/// The shape a `draw:enhanced-geometry`'s `draw:type` draws, when it is one WordCraft has.
pub(super) fn geometry_kind(t: &str) -> Option<ShapeKind> {
    match t {
        "rect" | "rectangle" | "ooxml-rect" | "flowchart-process" | "ooxml-flowChartProcess" => Some(ShapeKind::Rectangle),
        "round-rectangle" | "ooxml-roundRect" => Some(ShapeKind::RoundedRectangle),
        "ellipse" | "ooxml-ellipse" | "circle" | "flowchart-connector" => Some(ShapeKind::Ellipse),
        "diamond" | "ooxml-diamond" | "flowchart-decision" => Some(ShapeKind::Diamond),
        "isosceles-triangle" | "ooxml-triangle" => Some(ShapeKind::Triangle),
        "right-triangle" | "ooxml-rtTriangle" => Some(ShapeKind::RightTriangle),
        _ => None,
    }
}

impl Styles {
    /// A graphic property of style `style`, up its parent chain; drawing shapes (`shape`) then
    /// fall back to the default graphic style.
    pub(super) fn graphic<'a>(&'a self, style: Option<&'a str>, key: &str, shape: bool) -> Option<&'a str> {
        let chain = style.map(|s| self.chain(s)).unwrap_or_default();
        let from_chain = chain.iter().find_map(|(_, s)| get(&s.graphic, key));
        from_chain.or_else(|| if shape { self.map.get(DEFAULT_GRAPHIC).and_then(|s| get(&s.graphic, key)) } else { None })
    }

    /// How an object of graphic style `style` looks: a drawing shape (`shape`) or a frame.
    pub(super) fn look(&self, style: Option<&str>, shape: bool) -> Look {
        let g = |k: &str| self.graphic(style, k, shape);
        let color = |k: &str| g(k).and_then(model::parse_color);
        // A drawing shape is filled and outlined unless its style says otherwise; a frame shows
        // its background colour and border.
        let fill = match g("draw:fill") {
            Some("solid") => color("draw:fill-color"),
            Some(_) => None,
            None if shape => color("draw:fill-color"),
            None => g("fo:background-color").filter(|v| *v != "transparent").and_then(model::parse_color),
        };
        let (stroke, stroke_width) = if shape {
            match g("draw:stroke") {
                Some("none") => (None, 0.0),
                _ => {
                    let w = g("svg:stroke-width").and_then(length).unwrap_or(0.0).clamp(0.0, 1584.0);
                    // A zero width is a hairline.
                    (Some(color("svg:stroke-color").unwrap_or(Rgb(0, 0, 0))), if w > 0.0 { w } else { 0.5 })
                }
            }
        } else {
            match g("fo:border").and_then(border) {
                Some((c, w)) => (Some(c), w),
                None => (None, 0.0),
            }
        };
        let pad = g("fo:padding").and_then(length);
        let side = |k: &str| g(k).and_then(length).or(pad);
        let sides = [side("fo:padding-left"), side("fo:padding-top"), side("fo:padding-right"), side("fo:padding-bottom")];
        let inset = sides.iter().any(Option::is_some).then(|| sides.map(|v| (v.unwrap_or(0.0).clamp(0.0, 1584.0) * 12_700.0) as u32));
        let valign = match g("draw:textarea-vertical-align") {
            Some("middle") => VAlign::Center,
            Some("bottom") => VAlign::Bottom,
            Some(_) => VAlign::Top,
            // Drawing shapes centre their text by default; text frames start at the top.
            None if shape => VAlign::Center,
            None => VAlign::Top,
        };
        Look { fill, stroke, stroke_width, inset, valign }
    }

    /// Where the object of element attributes `a` floats; `None` when it sits in the line as a
    /// character (`text:anchor-type="as-char"`).
    pub(super) fn float(&self, a: &Attrs, shape: bool) -> Option<Float> {
        let style = get(a, "draw:style-name");
        let g = |k: &str| self.graphic(style, k, shape);
        let anchor = get(a, "text:anchor-type").or_else(|| g("text:anchor-type")).unwrap_or("paragraph");
        if anchor == "as-char" {
            return None;
        }
        let page = anchor == "page";
        let mut f = Float::default();
        f.wrap = match g("style:wrap") {
            // The model has one wrap per object: an object in the background draws behind the
            // text (and behind other objects after it), as LibreOffice shows it.
            Some("run-through") if g("style:run-through") == Some("background") => Wrap::BehindText,
            Some("run-through") => Wrap::InFrontOfText,
            _ if g("style:run-through") == Some("background") => Wrap::BehindText,
            Some("none") => Wrap::TopAndBottom,
            Some("left") => {
                f.wrap_text = WrapText::Left;
                Wrap::Square
            }
            Some("right") => {
                f.wrap_text = WrapText::Right;
                Wrap::Square
            }
            Some("dynamic") | Some("biggest") => {
                f.wrap_text = WrapText::Largest;
                Wrap::Square
            }
            _ => Wrap::Square,
        };
        f.h_rel = match g("style:horizontal-rel") {
            Some("page") => Anchor::Page,
            Some("page-content") => Anchor::Margin,
            Some("page-start-margin") => Anchor::LeftMargin,
            Some("page-end-margin") => Anchor::RightMargin,
            Some("char") => Anchor::Character,
            _ if page => Anchor::Page,
            _ => Anchor::Column,
        };
        f.v_rel = match g("style:vertical-rel") {
            Some("page") => Anchor::Page,
            Some("page-content") => Anchor::Margin,
            Some("line" | "char" | "text" | "baseline") => Anchor::Line,
            _ if page => Anchor::Page,
            _ => Anchor::Paragraph,
        };
        f.h_align = match g("style:horizontal-pos") {
            Some("left") => Some(FloatAlign::Start),
            Some("center") => Some(FloatAlign::Center),
            Some("right") => Some(FloatAlign::End),
            Some("inside") => Some(FloatAlign::Inside),
            Some("outside") => Some(FloatAlign::Outside),
            _ => None,
        };
        f.v_align = match g("style:vertical-pos") {
            Some("top") => Some(FloatAlign::Start),
            Some("middle") => Some(FloatAlign::Center),
            Some("bottom") => Some(FloatAlign::End),
            _ => None,
        };
        let at = |k: &str| get(a, k).and_then(length).map(clamp_len).unwrap_or(0.0);
        f.x = at("svg:x");
        f.y = at("svg:y");
        let dist = |k: &str| g(k).and_then(length).unwrap_or(0.0).clamp(0.0, 1584.0);
        f.dist = dist("fo:margin-left").max(dist("fo:margin-right"));
        f.dist_top = dist("fo:margin-top");
        f.dist_bottom = dist("fo:margin-bottom");
        Some(f)
    }
}
