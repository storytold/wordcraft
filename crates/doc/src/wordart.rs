//! WordArt: text effects on runs and text warps on shapes.
//!
//! Text effects are the run properties of [MS-DOCX] §2.6 (`w14:textFill`, `w14:textOutline`,
//! `w14:shadow`, `w14:glow`, `w14:reflection`): how a run's letters are filled and outlined and
//! what is drawn around them. A warp ("Transform", DrawingML `a:prstTxWarp`, ECMA-376
//! §20.1.9.19) bends a shape's whole text along a preset geometry. Lengths are points, angles
//! degrees, transparencies percent (0 = opaque).

use serde::{Deserialize, Serialize};

use crate::effects::{Glow, Shadow};
use crate::props::Rgb;

/// Most gradient stops kept.
pub const MAX_STOPS: usize = 10;
/// Widest text outline, points.
pub const MAX_OUTLINE: f32 = 20.0;
/// Largest reflection gap, points.
pub const MAX_REFLECTION_GAP: f32 = 100.0;
/// Most adjust values a warp keeps.
pub const MAX_ADJ: usize = 8;
/// Largest adjust value magnitude (DrawingML angles are 60000ths of a degree, fractions 100000ths).
pub const MAX_ADJ_VALUE: i64 = 100_000_000;

/// The WordArt effects of a run. `None` fields draw the text normally (its colour, no outline).
#[derive(Clone, Debug, PartialEq, Default, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct TextEffects {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fill: Option<TextFill>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub outline: Option<TextOutline>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub shadow: Option<Shadow>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub glow: Option<Glow>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reflection: Option<Reflection>,
}

/// How letters are filled.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum TextFill {
    /// Not filled (hollow letters: only their outline shows).
    None,
    Solid {
        color: Rgb,
        #[serde(default)]
        transparency: f32,
    },
    /// A linear gradient across the run's letters, at `angle` degrees (0 = left to right, 90 = top
    /// to bottom).
    Gradient {
        stops: Vec<GradStop>,
        #[serde(default)]
        angle: f32,
    },
}

/// A gradient stop at `pos` percent along the gradient.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GradStop {
    pub pos: f32,
    pub color: Rgb,
    #[serde(default)]
    pub transparency: f32,
}

/// The line around letters. `color: None` is explicitly no line.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct TextOutline {
    pub color: Option<Rgb>,
    pub width: f32,
    pub transparency: f32,
}

impl Default for TextOutline {
    fn default() -> Self {
        TextOutline { color: Some(Rgb::BLACK), width: 0.75, transparency: 0.0 }
    }
}

/// A mirror image of the letters below them that fades out: it starts at `transparency` and
/// fades to nothing over `size` percent of the letters' height, `distance` points below them.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Reflection {
    pub transparency: f32,
    pub size: f32,
    pub distance: f32,
    pub blur: f32,
}

impl Default for Reflection {
    fn default() -> Self {
        Reflection { transparency: 50.0, size: 50.0, distance: 0.0, blur: 0.5 }
    }
}

fn clamp(v: f32, lo: f32, hi: f32, dflt: f32) -> f32 {
    if v.is_finite() { v.clamp(lo, hi) } else { dflt }
}

impl TextFill {
    /// Finite, in range: at most [`MAX_STOPS`] stops, in order; a gradient without stops is no
    /// gradient (`None` back).
    pub fn sanitized(&self) -> Option<TextFill> {
        Some(match self {
            TextFill::None => TextFill::None,
            TextFill::Solid { color, transparency } => TextFill::Solid { color: *color, transparency: clamp(*transparency, 0.0, 100.0, 0.0) },
            TextFill::Gradient { stops, angle } => {
                let mut s: Vec<GradStop> = stops
                    .iter()
                    .take(MAX_STOPS)
                    .map(|g| GradStop { pos: clamp(g.pos, 0.0, 100.0, 0.0), color: g.color, transparency: clamp(g.transparency, 0.0, 100.0, 0.0) })
                    .collect();
                if s.is_empty() {
                    return None;
                }
                s.sort_by(|a, b| a.pos.total_cmp(&b.pos));
                TextFill::Gradient { stops: s, angle: clamp(*angle, -1e6, 1e6, 0.0).rem_euclid(360.0) }
            }
        })
    }
    /// One colour standing for the fill (a gradient's middle stop): what plain text, the caret
    /// and exports without gradients use.
    pub fn main_color(&self) -> Option<Rgb> {
        match self {
            TextFill::None => None,
            TextFill::Solid { color, .. } => Some(*color),
            TextFill::Gradient { stops, .. } => stops.get(stops.len() / 2).map(|s| s.color),
        }
    }
}

impl TextEffects {
    pub fn is_empty(&self) -> bool {
        *self == TextEffects::default()
    }
    /// Finite, in range; zero-size glows dropped.
    pub fn sanitized(&self) -> TextEffects {
        TextEffects {
            fill: self.fill.as_ref().and_then(TextFill::sanitized),
            outline: self.outline.map(|o| TextOutline {
                color: o.color,
                width: clamp(o.width, 0.0, MAX_OUTLINE, 0.75),
                transparency: clamp(o.transparency, 0.0, 100.0, 0.0),
            }),
            shadow: self.shadow.map(Shadow::sanitized),
            glow: self.glow.map(Glow::sanitized).filter(|g| g.size > 0.0),
            reflection: self.reflection.map(|r| Reflection {
                transparency: clamp(r.transparency, 0.0, 100.0, 50.0),
                size: clamp(r.size, 0.0, 100.0, 50.0),
                distance: clamp(r.distance, 0.0, MAX_REFLECTION_GAP, 0.0),
                blur: clamp(r.blur, 0.0, crate::effects::MAX_BLUR, 0.0),
            }),
        }
    }
    /// Apply every `Some` field of `patch` on top of `self` (style, then direct formatting).
    pub fn overlay(&mut self, patch: &TextEffects) {
        if patch.fill.is_some() {
            self.fill.clone_from(&patch.fill);
        }
        if patch.outline.is_some() {
            self.outline = patch.outline;
        }
        if patch.shadow.is_some() {
            self.shadow = patch.shadow;
        }
        if patch.glow.is_some() {
            self.glow = patch.glow;
        }
        if patch.reflection.is_some() {
            self.reflection = patch.reflection;
        }
    }
}

/// Our WordArt styles (Shape Format › WordArt Styles, Insert › WordArt): id and label. Their
/// colours come from the document's theme ([`art_style`]).
pub const ART_STYLES: [(&str, &str); 8] = [
    ("solid", "Solid Accent"),
    ("outlined", "Light with Outline"),
    ("hollow", "Hollow"),
    ("gradient", "Accent Gradient"),
    ("shadowed", "Dark with Shadow"),
    ("glowing", "Soft Glow"),
    ("reflected", "Accent with Reflection"),
    ("sunset", "Warm Gradient with Outline"),
];

/// The text effects of WordArt style `id` (see [`ART_STYLES`]) in theme colours `theme` (dk1,
/// lt1, dk2, lt2, accent1…6 as in [`crate::THEME_COLORS`]). `None` for an unknown id.
pub fn art_style(id: &str, theme: &[Rgb]) -> Option<TextEffects> {
    let slot = |i: usize| theme.get(i).or_else(|| crate::THEME_COLORS.get(i)).copied().unwrap_or(Rgb::BLACK);
    let (dark, light, accent, accent2) = (slot(2), slot(1), slot(4), slot(5));
    let solid = |c: Rgb| Some(TextFill::Solid { color: c, transparency: 0.0 });
    let line = |c: Rgb, w: f32| Some(TextOutline { color: Some(c), width: w, transparency: 0.0 });
    let grad = |a: Rgb, b: Rgb| {
        Some(TextFill::Gradient {
            stops: vec![GradStop { pos: 0.0, color: a, transparency: 0.0 }, GradStop { pos: 100.0, color: b, transparency: 0.0 }],
            angle: 90.0,
        })
    };
    let fx = match id {
        "solid" => TextEffects { fill: solid(accent), ..Default::default() },
        "outlined" => TextEffects { fill: solid(light), outline: line(accent, 1.0), ..Default::default() },
        "hollow" => TextEffects { fill: Some(TextFill::None), outline: line(dark, 1.0), ..Default::default() },
        "gradient" => TextEffects { fill: grad(accent, slot(7)), ..Default::default() },
        "shadowed" => TextEffects {
            fill: solid(dark),
            shadow: Some(Shadow { color: Rgb::BLACK, transparency: 55.0, blur: 3.0, distance: 3.0, angle: 45.0, rot_with_shape: false }),
            ..Default::default()
        },
        "glowing" => TextEffects {
            fill: solid(light),
            outline: line(accent, 0.5),
            glow: Some(Glow { color: accent, size: 6.0, transparency: 55.0 }),
            ..Default::default()
        },
        "reflected" => TextEffects { fill: solid(accent), reflection: Some(Reflection::default()), ..Default::default() },
        "sunset" => TextEffects { fill: grad(accent2, slot(8)), outline: line(dark, 0.5), ..Default::default() },
        _ => return None,
    };
    Some(fx)
}

/// Every DrawingML preset text warp (ECMA-376 §20.1.10.76 `ST_TextShapeType`). A warp read with
/// another name is dropped.
pub const WARP_PRESETS: [&str; 41] = [
    "textNoShape",
    "textPlain",
    "textStop",
    "textTriangle",
    "textTriangleInverted",
    "textChevron",
    "textChevronInverted",
    "textRingInside",
    "textRingOutside",
    "textArchUp",
    "textArchDown",
    "textCircle",
    "textButton",
    "textArchUpPour",
    "textArchDownPour",
    "textCirclePour",
    "textButtonPour",
    "textCurveUp",
    "textCurveDown",
    "textCanUp",
    "textCanDown",
    "textWave1",
    "textWave2",
    "textDoubleWave1",
    "textWave4",
    "textInflate",
    "textDeflate",
    "textInflateBottom",
    "textDeflateBottom",
    "textInflateTop",
    "textDeflateTop",
    "textDeflateInflate",
    "textDeflateInflateDeflate",
    "textFadeRight",
    "textFadeLeft",
    "textFadeUp",
    "textFadeDown",
    "textSlantUp",
    "textSlantDown",
    "textCascadeUp",
    "textCascadeDown",
];

/// The warps WordCraft draws bent (the others round-trip and draw flat), with menu labels.
pub const DRAWN_WARPS: [(&str, &str); 12] = [
    ("textArchUp", "Arch Up"),
    ("textArchDown", "Arch Down"),
    ("textCircle", "Circle"),
    ("textWave1", "Wave"),
    ("textSlantUp", "Slant Up"),
    ("textSlantDown", "Slant Down"),
    ("textInflate", "Inflate"),
    ("textDeflate", "Deflate"),
    ("textChevron", "Chevron Up"),
    ("textChevronInverted", "Chevron Down"),
    ("textTriangle", "Triangle Up"),
    ("textTriangleInverted", "Triangle Down"),
];

/// A shape's text warp: a preset and its adjust values (`a:avLst/a:gd`, name and `val`).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TextWarp {
    pub preset: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub adj: Vec<(String, i64)>,
}

impl TextWarp {
    /// A warp with preset `preset` (one of [`WARP_PRESETS`]); `None` for any other name.
    pub fn new(preset: &str) -> Option<TextWarp> {
        WARP_PRESETS.iter().find(|p| **p == preset).map(|p| TextWarp { preset: (*p).to_string(), adj: Vec::new() })
    }
    /// Add an adjust value (capped: at most [`MAX_ADJ`], short ASCII names, values within
    /// ±[`MAX_ADJ_VALUE`]); a repeated name replaces the earlier value.
    pub fn set_adj(&mut self, name: &str, val: i64) {
        if name.is_empty() || name.len() > 16 || !name.chars().all(|c| c.is_ascii_alphanumeric()) {
            return;
        }
        let val = val.clamp(-MAX_ADJ_VALUE, MAX_ADJ_VALUE);
        if let Some(slot) = self.adj.iter_mut().find(|(n, _)| n == name) {
            slot.1 = val;
        } else if self.adj.len() < MAX_ADJ {
            self.adj.push((name.to_string(), val));
        }
    }
    /// The adjust value `name`, if set.
    pub fn adj_value(&self, name: &str) -> Option<i64> {
        self.adj.iter().find(|(n, _)| n == name).map(|(_, v)| *v)
    }
    /// Valid: a known preset, capped adjust values (what files and commands are reduced to).
    pub fn sanitized(&self) -> Option<TextWarp> {
        let mut w = TextWarp::new(&self.preset)?;
        for (n, v) in &self.adj {
            w.set_adj(n, *v);
        }
        Some(w)
    }
    /// Does the warp leave the text as it is (`textNoShape`, `textPlain`)?
    pub fn is_plain(&self) -> bool {
        matches!(self.preset.as_str(), "textNoShape" | "textPlain")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hostile_values_are_reduced() {
        let fx = TextEffects {
            fill: Some(TextFill::Gradient {
                stops: (0..50).map(|i| GradStop { pos: if i == 0 { f32::NAN } else { 1e9 }, color: Rgb::BLACK, transparency: -5.0 }).collect(),
                angle: f32::INFINITY,
            }),
            outline: Some(TextOutline { color: None, width: 1e9, transparency: f32::NAN }),
            ..Default::default()
        }
        .sanitized();
        let Some(TextFill::Gradient { stops, angle }) = fx.fill else { panic!("a gradient") };
        assert_eq!(stops.len(), MAX_STOPS);
        assert!(stops.iter().all(|s| (0.0..=100.0).contains(&s.pos) && s.transparency == 0.0));
        assert_eq!(angle, 0.0);
        assert_eq!(fx.outline.map(|o| o.width), Some(MAX_OUTLINE));
        assert_eq!(TextFill::Gradient { stops: vec![], angle: 0.0 }.sanitized(), None);

        assert!(TextWarp::new("textMadeUp").is_none());
        let mut w = TextWarp::new("textArchUp").unwrap();
        for i in 0..20 {
            w.set_adj(&format!("adj{i}"), i64::MAX);
        }
        w.set_adj("bad name!", 1);
        assert_eq!(w.adj.len(), MAX_ADJ);
        assert!(w.adj.iter().all(|(_, v)| *v == MAX_ADJ_VALUE));
        for (id, _) in ART_STYLES {
            assert!(art_style(id, &[]).is_some(), "{id}");
        }
    }
}
