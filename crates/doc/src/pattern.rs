//! A pattern page background (Design › Page Background › Fill Effects › Pattern, the VML
//! `v:fill type="pattern"` Word writes): two colours in one of Word's preset patterns.
//!
//! The geometry of each preset is kept here, in a unit tile that both renderers repeat, so the
//! screen, the page and a PDF draw the same pattern.

use serde::{Deserialize, Serialize};

use crate::props::Rgb;

/// A pattern fill for the page background.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct PagePattern {
    /// The pattern's name (`v:fill/@pattern`: `pct20`, `ltDnDiag`, `smGrid`…).
    pub preset: String,
    /// The pattern's ink (`v:fill/@color`).
    pub fore: Rgb,
    /// The paper behind it (`v:fill/@color2`).
    pub back: Rgb,
    /// How much of the fill shows through, percent (100 = invisible), as Word's Fill Effects
    /// transparency.
    pub transparency: f32,
}

impl Default for PagePattern {
    fn default() -> Self {
        PagePattern { preset: "ltDnDiag".into(), fore: Rgb::BLACK, back: Rgb::WHITE, transparency: 0.0 }
    }
}

/// A pattern preset Word writes in `v:fill/@pattern`, with the label our pane shows.
pub struct PatternPreset {
    pub name: &'static str,
    pub label: &'static str,
}

/// The presets the pane offers and the file formats round-trip.
pub const PATTERN_PRESETS: &[PatternPreset] = &[
    PatternPreset { name: "pct5", label: "5%" },
    PatternPreset { name: "pct10", label: "10%" },
    PatternPreset { name: "pct20", label: "20%" },
    PatternPreset { name: "pct25", label: "25%" },
    PatternPreset { name: "pct30", label: "30%" },
    PatternPreset { name: "pct40", label: "40%" },
    PatternPreset { name: "pct50", label: "50%" },
    PatternPreset { name: "pct60", label: "60%" },
    PatternPreset { name: "pct70", label: "70%" },
    PatternPreset { name: "pct75", label: "75%" },
    PatternPreset { name: "pct80", label: "80%" },
    PatternPreset { name: "pct90", label: "90%" },
    PatternPreset { name: "ltHorz", label: "Light horizontal" },
    PatternPreset { name: "dkHorz", label: "Dark horizontal" },
    PatternPreset { name: "ltVert", label: "Light vertical" },
    PatternPreset { name: "dkVert", label: "Dark vertical" },
    PatternPreset { name: "ltDnDiag", label: "Light downward diagonal" },
    PatternPreset { name: "dkDnDiag", label: "Dark downward diagonal" },
    PatternPreset { name: "ltUpDiag", label: "Light upward diagonal" },
    PatternPreset { name: "dkUpDiag", label: "Dark upward diagonal" },
    PatternPreset { name: "smGrid", label: "Small grid" },
    PatternPreset { name: "lgGrid", label: "Large grid" },
    PatternPreset { name: "dotGrid", label: "Dotted grid" },
    PatternPreset { name: "smCheck", label: "Small checker" },
    PatternPreset { name: "lgCheck", label: "Large checker" },
];

impl PagePattern {
    /// Whether `preset` is one this build draws.
    pub fn known(preset: &str) -> bool {
        PATTERN_PRESETS.iter().any(|p| p.name == preset)
    }

    /// The preset it draws, defaulting to a light diagonal for a name from a file we don't
    /// know.
    pub fn safe_preset(&self) -> &'static str {
        match PATTERN_PRESETS.iter().find(|p| p.name == self.preset) {
            Some(p) => p.name,
            None => "ltDnDiag",
        }
    }

    /// Finite, in range: transparency 0–100.
    pub fn sanitized(&self) -> PagePattern {
        PagePattern {
            preset: self.safe_preset().to_string(),
            fore: self.fore,
            back: self.back,
            transparency: if self.transparency.is_finite() { self.transparency.clamp(0.0, 100.0) } else { 0.0 },
        }
    }

    /// The preset's marks in a unit tile: rectangles (x, y, w, h), line segments (x1, y1, x2, y2,
    /// width) and dots (x, y, radius), all fractions of the tile.
    pub fn tile(&self) -> PatternTile {
        let mut t = PatternTile::default();
        let preset = self.safe_preset();
        if let Some(pct) = preset.strip_prefix("pct").and_then(|p| p.parse::<u32>().ok()) {
            // 16 slots, filled in a scattered order so a scan line doesn't come out first.
            let count = ((pct.min(100) * 16 + 50) / 100) as usize;
            for k in 0..count {
                let i = (k * 5) % 16;
                let (x, y) = ((i % 4) as f32, (i / 4) as f32);
                t.dots.push([(x + 0.5) / 4.0, (y + 0.5) / 4.0, 0.11]);
            }
            return t;
        }
        match preset {
            "ltHorz" => t.strokes.push([0.0, 0.5, 1.0, 0.5, 0.06]),
            "dkHorz" => {
                t.strokes.push([0.0, 0.25, 1.0, 0.25, 0.12]);
                t.strokes.push([0.0, 0.75, 1.0, 0.75, 0.12]);
            }
            "ltVert" => t.strokes.push([0.5, 0.0, 0.5, 1.0, 0.06]),
            "dkVert" => {
                t.strokes.push([0.25, 0.0, 0.25, 1.0, 0.12]);
                t.strokes.push([0.75, 0.0, 0.75, 1.0, 0.12]);
            }
            "ltDnDiag" => t.strokes.push([0.0, 0.0, 1.0, 1.0, 0.06]),
            "dkDnDiag" => {
                t.strokes.push([0.0, 0.0, 1.0, 1.0, 0.12]);
                t.strokes.push([0.0, 1.0, 1.0, 2.0, 0.12]);
            }
            "ltUpDiag" => t.strokes.push([0.0, 1.0, 1.0, 0.0, 0.06]),
            "dkUpDiag" => {
                t.strokes.push([0.0, 1.0, 1.0, 0.0, 0.12]);
                t.strokes.push([0.0, 0.0, 1.0, -1.0, 0.12]);
            }
            "smGrid" => {
                t.strokes.push([0.5, 0.0, 0.5, 1.0, 0.06]);
                t.strokes.push([0.0, 0.5, 1.0, 0.5, 0.06]);
            }
            "lgGrid" => {
                t.strokes.push([0.25, 0.0, 0.25, 1.0, 0.08]);
                t.strokes.push([0.75, 0.0, 0.75, 1.0, 0.08]);
                t.strokes.push([0.0, 0.25, 1.0, 0.25, 0.08]);
                t.strokes.push([0.0, 0.75, 1.0, 0.75, 0.08]);
            }
            "dotGrid" => {
                for (x, y) in [(0.25, 0.25), (0.75, 0.25), (0.25, 0.75), (0.75, 0.75)] {
                    t.dots.push([x, y, 0.09]);
                }
                t.strokes.push([0.25, 0.0, 0.25, 1.0, 0.03]);
                t.strokes.push([0.75, 0.0, 0.75, 1.0, 0.03]);
                t.strokes.push([0.0, 0.25, 1.0, 0.25, 0.03]);
                t.strokes.push([0.0, 0.75, 1.0, 0.75, 0.03]);
            }
            "smCheck" => {
                for (x, y) in [(0.0, 0.0), (0.5, 0.25), (0.25, 0.5), (0.75, 0.75), (0.5, 0.0), (0.0, 0.25), (0.75, 0.5), (0.25, 0.75)] {
                    t.rects.push([x, y, 0.25, 0.25]);
                }
            }
            "lgCheck" => {
                t.rects.push([0.0, 0.0, 0.5, 0.5]);
                t.rects.push([0.5, 0.5, 0.5, 0.5]);
            }
            _ => t.strokes.push([0.0, 0.0, 1.0, 1.0, 0.06]),
        }
        t
    }
}

/// One tile of a pattern, in fractions of its side.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PatternTile {
    pub rects: Vec<[f32; 4]>,
    pub strokes: Vec<[f32; 5]>,
    pub dots: Vec<[f32; 3]>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_preset_has_marks_and_unknown_names_fall_back() {
        for p in PATTERN_PRESETS {
            let t = PagePattern { preset: p.name.into(), ..Default::default() }.tile();
            assert!(!(t.rects.is_empty() && t.strokes.is_empty() && t.dots.is_empty()), "{} draws nothing", p.name);
        }
        let unknown = PagePattern { preset: "hostile\"name".into(), ..Default::default() };
        assert_eq!(unknown.safe_preset(), "ltDnDiag");
        assert!(!unknown.tile().strokes.is_empty());
        assert_eq!(PagePattern { transparency: 250.0, ..Default::default() }.sanitized().transparency, 100.0);
        assert_eq!(PagePattern { transparency: f32::NAN, ..Default::default() }.sanitized().transparency, 0.0);
    }
}
