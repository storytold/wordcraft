//! DrawingML outline dashes, measured in multiples of the stroke width.

use serde::{Deserialize, Serialize};

pub const DASH_PERCENT: u32 = 100_000;
pub const MAX_DASH_STOPS: usize = 64;
const MIN_DASH_POINTS: f32 = 0.01;
const MAX_DASH_POINTS: f32 = 20_000.0;

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum LineDash {
    #[default]
    Solid,
    Dot,
    Dash,
    LongDash,
    DashDot,
    LongDashDot,
    LongDashDotDot,
    SystemDash,
    SystemDot,
    SystemDashDot,
    SystemDashDotDot,
    /// Dash/space pairs in 100000ths of the line width (`a:ds/@d`, `@sp`).
    Custom(Vec<(u32, u32)>),
}

impl LineDash {
    pub fn from_ooxml(value: &str) -> Self {
        match value {
            "dot" => Self::Dot,
            "dash" => Self::Dash,
            "lgDash" => Self::LongDash,
            "dashDot" => Self::DashDot,
            "lgDashDot" => Self::LongDashDot,
            "lgDashDotDot" => Self::LongDashDotDot,
            "sysDash" => Self::SystemDash,
            "sysDot" => Self::SystemDot,
            "sysDashDot" => Self::SystemDashDot,
            "sysDashDotDot" => Self::SystemDashDotDot,
            _ => Self::Solid,
        }
    }

    pub fn ooxml(&self) -> Option<&'static str> {
        Some(match self {
            Self::Solid => "solid",
            Self::Dot => "dot",
            Self::Dash => "dash",
            Self::LongDash => "lgDash",
            Self::DashDot => "dashDot",
            Self::LongDashDot => "lgDashDot",
            Self::LongDashDotDot => "lgDashDotDot",
            Self::SystemDash => "sysDash",
            Self::SystemDot => "sysDot",
            Self::SystemDashDot => "sysDashDot",
            Self::SystemDashDotDot => "sysDashDotDot",
            Self::Custom(_) => return None,
        })
    }

    pub fn is_solid(&self) -> bool {
        *self == Self::Solid
    }

    /// Page-point lengths for the painters. Bound hostile custom lists and tiny spans.
    pub fn pattern(&self, width: f32) -> Vec<f32> {
        let preset: &[f32] = match self {
            Self::Solid => return Vec::new(),
            Self::Dot => &[1.0, 3.0],
            Self::Dash => &[4.0, 3.0],
            Self::LongDash => &[8.0, 3.0],
            Self::DashDot => &[4.0, 3.0, 1.0, 3.0],
            Self::LongDashDot => &[8.0, 3.0, 1.0, 3.0],
            Self::LongDashDotDot => &[8.0, 3.0, 1.0, 3.0, 1.0, 3.0],
            Self::SystemDash => &[3.0, 1.0],
            Self::SystemDot => &[1.0, 1.0],
            Self::SystemDashDot => &[3.0, 1.0, 1.0, 1.0],
            Self::SystemDashDotDot => &[3.0, 1.0, 1.0, 1.0, 1.0, 1.0],
            Self::Custom(stops) => {
                return stops
                    .iter()
                    .take(MAX_DASH_STOPS)
                    .flat_map(|&(d, sp)| [d, sp])
                    .map(|v| dash_points(v as f32 / DASH_PERCENT as f32, width))
                    .collect();
            }
        };
        preset.iter().map(|v| dash_points(*v, width)).collect()
    }
}

fn dash_points(ratio: f32, width: f32) -> f32 {
    let width = if width.is_finite() { width.clamp(0.25, 200.0) } else { 0.75 };
    (ratio * width).clamp(MIN_DASH_POINTS, MAX_DASH_POINTS)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dash_units_follow_line_width() {
        assert_eq!(LineDash::Dash.pattern(1.0), [4.0, 3.0]);
        assert_eq!(LineDash::Dash.pattern(2.0), [8.0, 6.0]);
        assert_eq!(LineDash::Custom(vec![(200_000, 150_000)]).pattern(2.0), [4.0, 3.0]);
        assert!(LineDash::Solid.pattern(2.0).is_empty());
    }

    #[test]
    fn old_shapes_default_to_solid() {
        let extra: crate::connector::ShapeExtra = serde_json::from_str("{}").unwrap();
        assert!(extra.dash.is_solid());
        assert!(!serde_json::to_string(&extra).unwrap().contains("dash"));
    }

    #[test]
    fn custom_dashes_are_bounded() {
        let pattern = LineDash::Custom(vec![(0, u32::MAX); MAX_DASH_STOPS + 1]).pattern(f32::NAN);
        assert_eq!(pattern.len(), MAX_DASH_STOPS * 2);
        assert!(pattern.iter().all(|v| v.is_finite() && *v >= MIN_DASH_POINTS && *v <= MAX_DASH_POINTS));
    }
}
