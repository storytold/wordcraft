//! DrawingML pattern fills (ISO 29500 §20.1.8.47). Only light vertical is painted.
use crate::props::Rgb;
use serde::{Deserialize, Serialize};
use wordcraft_geom::Rect;

/// The supported DrawingML pattern presets.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum PatternKind {
    #[serde(rename = "ltVert")]
    LightVertical,
}

impl PatternKind {
    pub const fn ooxml(self) -> &'static str {
        match self {
            Self::LightVertical => "ltVert",
        }
    }
}

/// DrawingML color opacity scale (ISO29500 a:alpha).
pub const ALPHA_SCALE: u32 = 100_000;

/// One pattern color and its opacity in OOXML's 100000ths.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PatternColor {
    pub color: Rgb,
    pub alpha: u32,
}
impl PatternColor {
    pub fn opacity(self) -> f32 {
        self.alpha.min(ALPHA_SCALE) as f32 / ALPHA_SCALE as f32
    }
}

/// A pattern's colors remain separate so translucent bands do not overlap.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PatternFill {
    pub kind: PatternKind,
    pub foreground: PatternColor,
    pub background: PatternColor,
}

// Native Word 16.113.2 prints ltVert as 1 pt foreground, 3 pt background, page aligned.
const PITCH: f32 = 4.0;
const FOREGROUND_WIDTH: f32 = 1.0;
const MAX_BANDS: usize = 32_768;

impl PatternFill {
    pub fn color_at(self, x: f32) -> PatternColor {
        if x.rem_euclid(PITCH) < FOREGROUND_WIDTH { self.foreground } else { self.background }
    }

    /// Disjoint page-coordinate bands, bounded for hostile document sizes.
    pub fn bands(self, rect: Rect) -> Vec<(Rect, PatternColor)> {
        if ![rect.x, rect.y, rect.w, rect.h, rect.right(), rect.bottom()].iter().all(|v| v.is_finite()) || rect.w <= 0.0 || rect.h <= 0.0 {
            return Vec::new();
        }
        let mut result = Vec::new();
        let start = (rect.x / PITCH).floor() * PITCH;
        for i in 0..MAX_BANDS / 2 {
            let x = start + i as f32 * PITCH;
            if x >= rect.right() {
                break;
            }
            for (left, right, color) in [(x, x + FOREGROUND_WIDTH, self.foreground), (x + FOREGROUND_WIDTH, x + PITCH, self.background)] {
                let left = left.max(rect.x);
                let right = right.min(rect.right());
                if right > left {
                    result.push((Rect::new(left, rect.y, right - left, rect.h), color));
                }
            }
        }
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn pattern() -> PatternFill {
        PatternFill {
            kind: PatternKind::LightVertical,
            foreground: PatternColor { color: Rgb(155, 187, 89), alpha: 80_000 },
            background: PatternColor { color: Rgb::WHITE, alpha: 40_000 },
        }
    }
    #[test]
    fn bands_partition_and_keep_phase() {
        let p = pattern();
        let bands = p.bands(Rect::new(2.5, 3.0, 9.5, 20.0));
        assert!((bands.iter().map(|(r, _)| r.w).sum::<f32>() - 9.5).abs() < 0.0001);
        for pair in bands.windows(2) {
            assert!((pair[0].0.right() - pair[1].0.x).abs() < 0.0001);
        }
        assert_eq!(bands[0].1, p.background);
        assert_eq!(bands[1].0.x, 4.0);
        assert_eq!(bands[1].0.w, 1.0);
        assert_eq!(bands[1].1, p.foreground);
        assert_eq!(p.color_at(-0.5), p.background);
        assert_eq!(p.color_at(-3.5), p.foreground);
    }
    #[test]
    fn missing_pattern_is_default() {
        let extra: crate::connector::ShapeExtra = serde_json::from_str("{}").unwrap();
        assert!(extra.pattern.is_none());
        assert!(extra.is_empty());
    }
    #[test]
    fn hostile_bounds_are_bounded() {
        let p = pattern();
        assert!(p.bands(Rect { x: f32::NAN, y: 0.0, w: 10.0, h: 10.0 }).is_empty());
        assert!(p.bands(Rect::new(0.0, 0.0, -1.0, 10.0)).is_empty());
        assert!(p.bands(Rect::new(0.0, 0.0, 1e9, 10.0)).len() <= MAX_BANDS);
        assert_eq!(PatternColor { color: Rgb::WHITE, alpha: u32::MAX }.opacity(), 1.0);
    }
}
