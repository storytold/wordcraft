//! Shape fills: what paints a shape's, text box's or canvas's interior, and a shape outline's
//! gradient (DrawingML `a:solidFill` / `a:gradFill`, ECMA-376 §20.1.8.54 and §20.1.8.33; VML
//! `v:fill`).
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
    Solid { color: Rgb, transparency: f32 },
    Gradient(Arc<Gradient>),
}

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
    /// The one colour standing for the fill where only one fits: the solid colour, or a
    /// gradient's middle stop.
    pub fn main_color(&self) -> Rgb {
        match self {
            Fill::Solid { color, .. } => *color,
            Fill::Gradient(g) => g.stops.get(g.stops.len() / 2).map_or(Rgb::BLACK, |s| s.color),
        }
    }
    /// Finite, in range: at most [`MAX_STOPS`] stops, in order, positions and transparencies
    /// 0–100%, angles 0–360°. A gradient with one stop is that stop's solid colour; with none,
    /// no fill (`None`).
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
        }
    }
}

impl<'de> Deserialize<'de> for Fill {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Fill, D::Error> {
        Ok(match AnyForm::deserialize(d)? {
            AnyForm::Plain(color) => Fill::solid(color),
            AnyForm::Tagged(Tagged::Solid { color, transparency }) => Fill::Solid { color, transparency },
            AnyForm::Tagged(Tagged::Gradient(g)) => Fill::Gradient(Arc::new(g)),
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
