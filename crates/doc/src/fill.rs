//! Shape fills: what paints a shape's, text box's or canvas's interior, and a shape outline's
//! gradient (DrawingML `a:solidFill` / `a:gradFill` / `a:blipFill`, ECMA-376 §20.1.8.54,
//! §20.1.8.33 and §20.1.8.14; VML `v:fill`).
//!
//! A gradient is kept as the file states it (stops, linear angle or path focus, tile rectangle,
//! `rotWithShape`, flip) so saving writes it back; drawing approximates rectangular and
//! shape-following paths as circular. Transparencies are percent (0 = opaque); rectangles are
//! percent insets (left, top, right, bottom) from the shape's edges.

use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::sync::Arc;

use crate::props::Rgb;
#[cfg(test)]
use crate::wordart::MAX_STOPS;
use crate::wordart::{GradStop, clamp, sanitized_stops};

/// A shape's fill. In `.json` documents an opaque solid fill is written `[r, g, b]` as before
/// fills had transparency or gradients; the rest are tagged (`{"type": "gradient", ...}`).
#[derive(Clone, Debug, PartialEq)]
pub enum Fill {
    Solid {
        color: Rgb,
        transparency: f32,
    },
    Gradient(Arc<Gradient>),
    /// A picture stretched over the shape and clipped to its outline.
    Picture(Arc<PictureFill>),
}

/// A picture fill (`a:blipFill` in a shape's properties).
///
/// The picture is stretched over the shape's frame, inset by `stretch` (`a:stretch/a:fillRect`),
/// and clipped to the shape's outline. A tiled picture (`a:tile`) keeps its tiling for saving
/// but is drawn stretched like the others: WordCraft doesn't repeat it across the shape.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PictureFill {
    /// Key in `Document::media`.
    pub media: String,
    /// `a:srcRect`: the part of the picture used, as fractions 0..1 cut off its left, top, right
    /// and bottom (as a picture's crop).
    #[serde(default)]
    pub crop: [f32; 4],
    /// `a:fillRect`: where the picture is stretched to, percent insets from the shape's edges
    /// (negative: past them, cut off by the outline).
    #[serde(default)]
    pub stretch: [f32; 4],
    /// Transparency, percent (`a:alphaModFix`; 0 = opaque).
    #[serde(default)]
    pub transparency: f32,
    /// Turns with the shape (`rotWithShape`).
    #[serde(default)]
    pub rot_with_shape: bool,
    /// `a:tile`, kept for saving (drawn stretched).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tile: Option<PictureTile>,
}

/// How a tiled picture fill repeats (`a:tile`, ECMA-376 §20.1.8.58): kept as the file states it.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PictureTile {
    /// Offset of the first tile, points.
    #[serde(default)]
    pub tx: f32,
    #[serde(default)]
    pub ty: f32,
    /// Tile scale, percent of the picture's size.
    #[serde(default = "hundred")]
    pub sx: f32,
    #[serde(default = "hundred")]
    pub sy: f32,
    #[serde(default, skip_serializing_if = "GradientFlip::is_none")]
    pub flip: GradientFlip,
    /// Where the tiles are aligned (`algn`: `tl`, `ctr`…); empty for the default.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub align: String,
}

fn hundred() -> f32 {
    100.0
}

/// Longest media key a picture fill keeps (hostile files and commands).
const MAX_MEDIA_KEY: usize = 256;

/// A gradient fill.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Gradient {
    pub stops: Vec<GradStop>,
    pub kind: GradientKind,
    /// `a:tileRect`: where the gradient's area sits relative to the shape (percent insets).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tile: Option<[f32; 4]>,
    /// Turns with the shape (`rotWithShape`).
    #[serde(default)]
    pub rot_with_shape: bool,
    #[serde(default, skip_serializing_if = "GradientFlip::is_none")]
    pub flip: GradientFlip,
}

/// How a gradient runs.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum GradientKind {
    /// Along a line at `angle` degrees (0 = left to right, 90 = top to bottom). `scaled`: the
    /// angle is scaled with the shape's aspect ratio.
    Linear {
        angle: f32,
        #[serde(default)]
        scaled: bool,
    },
    /// Outwards from `focus` (percent insets from the shape's edges, `a:fillToRect`).
    Path { shape: PathShape, focus: [f32; 4] },
}

/// A path gradient's outline (`a:path/@path`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum PathShape {
    Circle,
    Rect,
    Shape,
}

/// How the gradient's tile is mirrored when tiled (`a:gradFill/@flip`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum GradientFlip {
    #[default]
    None,
    X,
    Y,
    Xy,
}

impl GradientFlip {
    pub fn is_none(&self) -> bool {
        *self == GradientFlip::None
    }
}

/// Largest rectangle inset kept, percent (hostile files).
const MAX_INSET: f32 = 1000.0;

fn rect(r: [f32; 4]) -> [f32; 4] {
    r.map(|v| clamp(v, -MAX_INSET, MAX_INSET, 0.0))
}

impl Fill {
    /// An opaque solid fill.
    pub fn solid(color: Rgb) -> Fill {
        Fill::Solid { color, transparency: 0.0 }
    }
    /// The one colour standing for the fill where only one fits: the solid colour, a
    /// gradient's middle stop, or white for a picture (the paper it's printed on).
    pub fn main_color(&self) -> Rgb {
        match self {
            Fill::Solid { color, .. } => *color,
            Fill::Gradient(g) => g.stops.get(g.stops.len() / 2).map_or(Rgb::BLACK, |s| s.color),
            Fill::Picture(_) => Rgb::WHITE,
        }
    }
    /// Finite, in range: at most [`MAX_STOPS`] stops, in order, positions and transparencies
    /// 0–100%, angles 0–360°. A gradient with one stop is that stop's solid colour; with none,
    /// no fill (`None`). A picture's crop leaves some of it (each side 0..1, together under 1),
    /// its insets are bounded, and one with no media key (or an absurd one) is no fill.
    pub fn sanitized(&self) -> Option<Fill> {
        Some(match self {
            Fill::Solid { color, transparency } => Fill::Solid { color: *color, transparency: clamp(*transparency, 0.0, 100.0, 0.0) },
            Fill::Gradient(g) => {
                let stops = sanitized_stops(&g.stops);
                match stops[..] {
                    [] => return None,
                    [s] => return Some(Fill::Solid { color: s.color, transparency: s.transparency }),
                    _ => {}
                }
                let kind = match g.kind {
                    GradientKind::Linear { angle, scaled } => {
                        GradientKind::Linear { angle: clamp(angle, -1e6, 1e6, 0.0).rem_euclid(360.0) % 360.0, scaled }
                    }
                    GradientKind::Path { shape, focus } => GradientKind::Path { shape, focus: rect(focus) },
                };
                Fill::Gradient(Arc::new(Gradient { stops, kind, tile: g.tile.map(rect), rot_with_shape: g.rot_with_shape, flip: g.flip }))
            }
            Fill::Picture(p) => {
                if p.media.is_empty() || p.media.len() > MAX_MEDIA_KEY {
                    return None;
                }
                let mut crop = p.crop.map(|v| clamp(v, 0.0, 1.0, 0.0));
                // Opposite sides together leave at least a sliver of the picture.
                for (a, b) in [(0, 2), (1, 3)] {
                    let sum = crop[a] + crop[b];
                    if sum > 0.99 {
                        let k = 0.99 / sum;
                        crop[a] *= k;
                        crop[b] *= k;
                    }
                }
                let tile = p.tile.as_ref().map(|t| PictureTile {
                    tx: clamp(t.tx, -1e6, 1e6, 0.0),
                    ty: clamp(t.ty, -1e6, 1e6, 0.0),
                    sx: clamp(t.sx, -MAX_INSET * 10.0, MAX_INSET * 10.0, 100.0),
                    sy: clamp(t.sy, -MAX_INSET * 10.0, MAX_INSET * 10.0, 100.0),
                    flip: t.flip,
                    align: if t.align.len() <= 8 { t.align.clone() } else { String::new() },
                });
                Fill::Picture(Arc::new(PictureFill {
                    media: p.media.clone(),
                    crop,
                    stretch: rect(p.stretch),
                    transparency: clamp(p.transparency, 0.0, 100.0, 0.0),
                    rot_with_shape: p.rot_with_shape,
                    tile,
                }))
            }
        })
    }
}

/// The tagged `.json` form of fills other than opaque solid ones.
#[derive(Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
enum Tagged {
    Solid {
        color: Rgb,
        #[serde(default)]
        transparency: f32,
    },
    Gradient(Gradient),
    Picture(PictureFill),
}

#[derive(Deserialize)]
#[serde(untagged)]
enum AnyForm {
    Plain(Rgb),
    Tagged(Tagged),
}

impl Serialize for Fill {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        match self {
            Fill::Solid { color, transparency: 0.0 } => color.serialize(s),
            Fill::Solid { color, transparency } => Tagged::Solid { color: *color, transparency: *transparency }.serialize(s),
            Fill::Gradient(g) => Tagged::Gradient(Gradient::clone(g)).serialize(s),
            Fill::Picture(p) => Tagged::Picture(PictureFill::clone(p)).serialize(s),
        }
    }
}

impl<'de> Deserialize<'de> for Fill {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Fill, D::Error> {
        Ok(match AnyForm::deserialize(d)? {
            AnyForm::Plain(color) => Fill::solid(color),
            AnyForm::Tagged(Tagged::Solid { color, transparency }) => Fill::Solid { color, transparency },
            AnyForm::Tagged(Tagged::Gradient(g)) => Fill::Gradient(Arc::new(g)),
            AnyForm::Tagged(Tagged::Picture(p)) => Fill::Picture(Arc::new(p)),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stop(pos: f32, color: Rgb, transparency: f32) -> GradStop {
        GradStop { pos, color, transparency }
    }

    fn grad(stops: Vec<GradStop>, kind: GradientKind) -> Fill {
        Fill::Gradient(Arc::new(Gradient { stops, kind, tile: None, rot_with_shape: true, flip: GradientFlip::None }))
    }

    #[test]
    fn opaque_solid_fills_are_written_as_before_and_old_documents_load() {
        let f = Fill::solid(Rgb(1, 2, 3));
        assert_eq!(serde_json::to_string(&f).ok().as_deref(), Some("[1,2,3]"));
        assert_eq!(serde_json::from_str::<Fill>("[1,2,3]").ok(), Some(f));
    }

    #[test]
    fn transparent_and_gradient_fills_round_trip_tagged() {
        let fills = [
            Fill::Solid { color: Rgb::WHITE, transparency: 70.0 },
            grad(vec![stop(0.0, Rgb(9, 9, 9), 0.0), stop(100.0, Rgb::WHITE, 50.0)], GradientKind::Linear { angle: 90.0, scaled: true }),
            Fill::Gradient(Arc::new(Gradient {
                stops: vec![stop(0.0, Rgb::BLACK, 0.0), stop(100.0, Rgb::WHITE, 0.0)],
                kind: GradientKind::Path { shape: PathShape::Rect, focus: [50.0, 50.0, 50.0, 50.0] },
                tile: Some([-10.0, 0.0, 0.0, 0.0]),
                rot_with_shape: false,
                flip: GradientFlip::Xy,
            })),
        ];
        for f in fills {
            let json = serde_json::to_string(&f).unwrap_or_default();
            assert!(json.contains("\"type\""), "{json}");
            assert_eq!(serde_json::from_str::<Fill>(&json).ok(), Some(f), "{json}");
        }
    }

    #[test]
    fn hostile_fills_are_sanitized() {
        let many: Vec<GradStop> = (0..30).map(|i| stop(100.0 - i as f32, Rgb::BLACK, f32::NAN)).collect();
        let Some(Fill::Gradient(g)) = grad(many, GradientKind::Linear { angle: -450.0, scaled: false }).sanitized() else { panic!() };
        assert_eq!(g.stops.len(), MAX_STOPS);
        assert!(g.stops.windows(2).all(|w| w[0].pos <= w[1].pos) && g.stops.iter().all(|s| s.transparency == 0.0));
        assert_eq!(g.kind, GradientKind::Linear { angle: 270.0, scaled: false });
        let path = grad(
            vec![stop(150.0, Rgb::BLACK, 200.0), stop(-5.0, Rgb::WHITE, 0.0)],
            GradientKind::Path { shape: PathShape::Circle, focus: [f32::NAN, 1e9, -1e9, 5.0] },
        );
        let Some(Fill::Gradient(g)) = path.sanitized() else { panic!() };
        assert_eq!((g.stops[0].pos, g.stops[1].pos, g.stops[1].transparency), (0.0, 100.0, 100.0));
        assert_eq!(g.kind, GradientKind::Path { shape: PathShape::Circle, focus: [0.0, MAX_INSET, -MAX_INSET, 5.0] });
        assert_eq!(
            grad(vec![stop(0.0, Rgb::WHITE, 30.0)], GradientKind::Linear { angle: 0.0, scaled: false }).sanitized(),
            Some(Fill::Solid { color: Rgb::WHITE, transparency: 30.0 })
        );
        assert_eq!(grad(vec![], GradientKind::Linear { angle: 0.0, scaled: false }).sanitized(), None);
        assert_eq!(Fill::Solid { color: Rgb::BLACK, transparency: f32::NAN }.sanitized(), Some(Fill::solid(Rgb::BLACK)));
    }

    fn picture(media: &str) -> PictureFill {
        PictureFill { media: media.into(), crop: [0.0; 4], stretch: [0.0; 4], transparency: 0.0, rot_with_shape: true, tile: None }
    }

    #[test]
    fn picture_fills_round_trip_tagged() {
        let p = PictureFill {
            crop: [0.1, 0.0, 0.2, 0.0],
            stretch: [0.0, 0.0, -7.5, 0.0],
            transparency: 25.0,
            tile: Some(PictureTile { tx: 1.0, ty: 2.0, sx: 50.0, sy: 50.0, flip: GradientFlip::X, align: "tl".into() }),
            ..picture("banner.png")
        };
        let f = Fill::Picture(Arc::new(p));
        let json = serde_json::to_string(&f).unwrap_or_default();
        assert!(json.contains("\"type\":\"picture\""), "{json}");
        assert_eq!(serde_json::from_str::<Fill>(&json).ok(), Some(f.clone()), "{json}");
        assert_eq!(f.main_color(), Rgb::WHITE);
        let bare: Fill = serde_json::from_str(r#"{"type": "picture", "media": "a.png"}"#).unwrap_or(Fill::solid(Rgb::BLACK));
        assert_eq!(bare, Fill::Picture(Arc::new(PictureFill { rot_with_shape: false, ..picture("a.png") })));
    }

    #[test]
    fn hostile_picture_fills_are_sanitized() {
        let p = PictureFill {
            crop: [0.9, f32::NAN, 0.9, -3.0],
            stretch: [f32::INFINITY, -1e9, 1e9, 5.0],
            transparency: 400.0,
            tile: Some(PictureTile { tx: f32::NAN, sx: 1e12, align: "x".repeat(100), ..Default::default() }),
            ..picture("p.png")
        };
        let Some(Fill::Picture(q)) = Fill::Picture(Arc::new(p)).sanitized() else { panic!() };
        assert!(q.crop[0] + q.crop[2] <= 0.99 + 1e-6 && q.crop[1] == 0.0 && q.crop[3] == 0.0, "{:?}", q.crop);
        assert_eq!(q.stretch, [0.0, -MAX_INSET, MAX_INSET, 5.0]);
        assert_eq!(q.transparency, 100.0);
        let t = q.tile.clone().unwrap_or_default();
        assert_eq!((t.tx, t.sx, t.align.as_str()), (0.0, MAX_INSET * 10.0, ""));
        assert_eq!(Fill::Picture(Arc::new(picture(""))).sanitized(), None);
        assert_eq!(Fill::Picture(Arc::new(picture(&"k".repeat(MAX_MEDIA_KEY + 1)))).sanitized(), None);
    }

    #[test]
    fn main_colour_is_the_middle_stop() {
        let g = grad(
            vec![stop(0.0, Rgb(1, 0, 0), 0.0), stop(50.0, Rgb(2, 0, 0), 0.0), stop(100.0, Rgb(3, 0, 0), 0.0)],
            GradientKind::Linear { angle: 0.0, scaled: false },
        );
        assert_eq!(g.main_color(), Rgb(2, 0, 0));
        assert_eq!(Fill::solid(Rgb::WHITE).main_color(), Rgb::WHITE);
    }
}
