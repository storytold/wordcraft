//! Charts and SmartArt diagrams: drawings shown but not edited, kept as neutral items ready to draw.

use serde::{Deserialize, Serialize};

use crate::para::ShapeKind;
use crate::props::Rgb;

/// A drawing shown but not edited: a chart or a SmartArt diagram, as items ready to draw.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Graphic {
    pub kind: GraphicKind,
    /// What to draw, in points from the object's top-left corner.
    #[serde(default)]
    pub items: Vec<GraphicItem>,
    /// The size (points) the items were built for. Drawn scaled to the object's rectangle when
    /// that differs (layout shrinks oversized inline objects); 0 means the rectangle's own size.
    #[serde(default)]
    pub w: f32,
    #[serde(default)]
    pub h: f32,
    /// Where the graphic was read from, so saving writes the chart or diagram itself back (not
    /// only these items). `None` for graphics made some other way.
    #[serde(skip)]
    pub source: Option<std::sync::Arc<Embedded>>,
}

/// What an object read from a file needs to be written back as it was: its own markup in the
/// source format and the package parts that markup refers to. Never edited; WordCraft only moves
/// and resizes the object around it. The parts themselves (bytes, content type and their own
/// relationships) are kept in [`crate::Document::passthrough`] under their part names.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Embedded {
    /// The object's markup as a standalone XML element with its namespace declarations (DOCX:
    /// the `a:graphic` of a chart or SmartArt diagram, the `w:object` of an OLE object).
    pub xml: String,
    /// The relationships it uses, by the ids in `xml` (or in its parts, for a SmartArt drawing).
    pub refs: Vec<EmbeddedRef>,
}

/// One relationship an [`Embedded`] object uses.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct EmbeddedRef {
    /// The relationship id the markup uses.
    pub id: String,
    /// Relationship type (a URI).
    pub kind: String,
    /// The part name (a `Document::passthrough` key), or the URL of an external target.
    pub target: String,
    pub external: bool,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum GraphicKind {
    #[default]
    Chart,
    Diagram,
}

impl GraphicKind {
    /// What the graphic is, in words: the alt text of one that has none of its own.
    pub fn noun(self) -> &'static str {
        match self {
            GraphicKind::Chart => "chart",
            GraphicKind::Diagram => "diagram",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum GraphicItem {
    /// A preset shape filling `rect` ([x, y, w, h]).
    Shape {
        rect: [f32; 4],
        kind: ShapeKind,
        fill: Option<Rgb>,
        stroke: Option<Rgb>,
        #[serde(default)]
        stroke_width: f32,
    },
    /// A path: polygons, pie slices, line series.
    Path {
        segs: Vec<PathSeg>,
        fill: Option<Rgb>,
        stroke: Option<Rgb>,
        #[serde(default)]
        stroke_width: f32,
    },
    /// One line of text in `rect`, aligned horizontally by `align`, centred vertically.
    Text {
        rect: [f32; 4],
        text: String,
        size: f32,
        color: Rgb,
        #[serde(default)]
        bold: bool,
        #[serde(default)]
        align: TextAlign,
        #[serde(default)]
        font: Option<String>,
    },
    /// A picture (`Document::media` key) stretched into `rect`.
    Image { rect: [f32; 4], media: String },
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum TextAlign {
    Left,
    #[default]
    Center,
    Right,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum PathSeg {
    Move(f32, f32),
    Line(f32, f32),
    Cubic(f32, f32, f32, f32, f32, f32),
    Close,
}
