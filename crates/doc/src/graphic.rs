//! Charts, SmartArt diagrams and native ink: drawings kept as neutral items ready to draw. Charts and SmartArt
//! WordCraft can edit also keep their model ([`crate::chart::ChartSpec`],
//! [`crate::smart_art::SmartArtSpec`]); the rest are shown, not edited.

use serde::{Deserialize, Serialize};

use crate::para::ShapeKind;
use crate::props::Rgb;

/// A chart, SmartArt diagram or native ink preview, as items ready to draw (and, for an editable chart or SmartArt
/// graphic, its model).
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ink_frame: Option<[f32; 4]>,
    /// Per-axis fitting scales already baked into native circular pen widths/taps.
    /// Later resizing composes these axes before averaging, rather than multiplying
    /// arithmetic means (which would change thickness after DOCX save/reopen).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ink_scale: Option<[f64; 2]>,
    /// Native ink's picture fallback (a `Document::media` key), stretched to the outer
    /// object frame by exporters that cannot represent the decoded vectors. Not drawn
    /// alongside `items`; kept independently so successful decoding does not lose it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ink_fallback: Option<String>,
    /// Modeled picture properties for a fallback reconstructed when its original markup cannot
    /// be retained. The original markup remains authoritative whenever it is available.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ink_fallback_style: Option<InkFallbackStyle>,
    /// Fallback markup could not be retained in full and may contain additional metadata. Saving
    /// must not silently discard it, whether or not the picture itself was usable.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub ink_fallback_unretained: bool,
    /// Where the graphic was read from, so saving writes the original drawing itself back (not
    /// only these items). `None` for graphics made some other way.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<std::sync::Arc<Embedded>>,
    /// A SmartArt graphic WordCraft can edit (one it made, or one read back exactly as it wrote
    /// it): saving writes the diagram parts from it, and the items are drawn from it. See
    /// [`crate::smart_art`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub smart_art: Option<std::sync::Arc<crate::smart_art::SmartArtSpec>>,
    /// A chart WordCraft can edit (one it made, or one read back that holds nothing more): saving
    /// writes the chart part from it, and the items are drawn from it. See [`crate::chart`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub chart: Option<std::sync::Arc<crate::chart::ChartSpec>>,
}

/// Picture properties needed to reconstruct a native-ink preview from its media.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct InkFallbackStyle {
    pub crop: [f32; 4],
    pub fx: crate::picfx::PicFx,
}

impl Graphic {
    /// Width multiplier for a further fit. Native circular pens average the composed
    /// per-axis scales once; other graphics and older previews retain their mean policy.
    pub fn ink_width_scale(&self, sx: f32, sy: f32) -> f32 {
        let fallback = (sx + sy) / 2.0;
        let Some([x, y]) = self.ink_scale.filter(|_| self.kind == GraphicKind::Ink) else { return fallback };
        let total = x + y;
        if !x.is_finite() || !y.is_finite() || x < 0.0 || y < 0.0 || !total.is_finite() || total <= 0.0 {
            return fallback;
        }
        let scale = (x / total) * f64::from(sx) + (y / total) * f64::from(sy);
        if scale.is_finite() && scale >= 0.0 && scale <= f64::from(f32::MAX) { scale as f32 } else { fallback }
    }
}

/// What an object read from a file needs to be written back as it was: its own markup in the
/// source format and the package parts that markup refers to. Never edited; WordCraft only moves
/// and resizes the object around it. The parts themselves (bytes, content type and their own
/// relationships) are kept in [`crate::Document::passthrough`] under their part names.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Embedded {
    /// The object's markup as a standalone XML element with its namespace declarations (DOCX:
    /// the `a:graphic` of a chart, SmartArt diagram or native ink, an ink `mc:AlternateContent`,
    /// the `w:object` of an OLE object, or the `w:pict` of a signature line).
    pub xml: String,
    /// The relationships it uses, by the ids in `xml` (or in its parts, for a SmartArt drawing).
    pub refs: Vec<EmbeddedRef>,
}

impl Embedded {
    /// The root element of an ODF embedded object's markup: its files are kept in
    /// [`crate::Document::passthrough`] for the ODT writer; DOCX shows it as its picture.
    pub const ODF_OBJECT: &'static str = "<wordcraft:odf-object";

    /// Is it an ODF embedded object (a spreadsheet in an ODT), not Word markup?
    pub fn is_odf_object(&self) -> bool {
        self.xml.trim_start().starts_with(Self::ODF_OBJECT)
    }
}

/// One relationship an [`Embedded`] object uses.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
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
    /// Word's native InkML: supported strokes are drawn as vectors, otherwise as its fallback
    /// picture. The original part is kept unchanged for saving.
    Ink,
}

impl GraphicKind {
    /// What the graphic is, in words: the alt text of one that has none of its own.
    pub fn noun(self) -> &'static str {
        match self {
            GraphicKind::Chart => "chart",
            GraphicKind::Diagram => "diagram",
            GraphicKind::Ink => "ink",
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
    /// A native ink nib swept into a compound, nonzero-filled outline. Painting the entire
    /// outline once avoids dark joins and self-overlaps in a translucent pen stroke.
    InkOutline {
        /// Closed local-coordinate polygons, already bounded by the InkML decoder.
        segs: Vec<PathSeg>,
        /// Resolved brush or sample-channel color.
        color: Rgb,
        /// Opacity in 0..1; original InkML transparency remains preserved separately.
        alpha: f32,
        #[serde(default)]
        mask_pen: bool,
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn native_widths_compose_axis_scales_and_ignore_invalid_metadata() {
        let mut graphic = Graphic { kind: GraphicKind::Ink, ink_scale: Some([10.0, 5.0]), ..Default::default() };
        assert!((15.0 * graphic.ink_width_scale(2.0, 1.0) - 25.0).abs() < 1e-5);
        graphic.kind = GraphicKind::Chart;
        assert_eq!(graphic.ink_width_scale(2.0, 1.0), 1.5);
        graphic.kind = GraphicKind::Ink;
        for scale in [None, Some([f64::NAN, 1.0]), Some([f64::INFINITY, 1.0]), Some([-1.0, 2.0]), Some([0.0, 0.0]), Some([f64::MAX; 2])] {
            graphic.ink_scale = scale;
            assert_eq!(graphic.ink_width_scale(2.0, 1.0), 1.5);
        }
    }
}
