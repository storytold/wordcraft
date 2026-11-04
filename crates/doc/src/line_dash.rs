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

/// A DrawingML outline's compound type (`a:ln/@cmpd`, ECMA-376 §20.1.10.15): one line, or
/// two or three parallel lines sharing the outline's total width.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum CompoundLine {
    #[default]
    Single,
    Double,
    /// A thick line outside a thin one.
    ThickThin,
    /// A thin line outside a thick one.
    ThinThick,
    /// A thick line between two thin ones.
    Triple,
}

impl CompoundLine {
    pub fn from_ooxml(value: &str) -> Self {
        match value {
            "dbl" => Self::Double,
            "thickThin" => Self::ThickThin,
            "thinThick" => Self::ThinThick,
            "tri" => Self::Triple,
            _ => Self::Single,
        }
    }

    pub fn to_ooxml(self) -> &'static str {
        match self {
            Self::Single => "sng",
            Self::Double => "dbl",
            Self::ThickThin => "thickThin",
            Self::ThinThick => "thinThick",
            Self::Triple => "tri",
        }
    }

    pub fn is_single(&self) -> bool {
        *self == Self::Single
    }

    /// The parallel lines of an outline `width` points wide, as (offset of the line's centre
    /// from the outline's centre, positive outward; the line's width), outermost first. The
    /// lines and the gaps between them share the width: Double is thirds, thick-thin quarters
    /// (the thick line two of them), Triple sixths (thin, gap, thick of two, gap, thin).
    pub fn lines(self, width: f32) -> Vec<(f32, f32)> {
        let w = width;
        let half = w / 2.0;
        match self {
            Self::Single => vec![(0.0, w)],
            Self::Double => {
                let u = w / 3.0;
                vec![(u, u), (-u, u)]
            }
            Self::ThickThin => {
                let u = w / 4.0;
                vec![(half - u, 2.0 * u), (-half + u / 2.0, u)]
            }
            Self::ThinThick => {
                let u = w / 4.0;
                vec![(half - u / 2.0, u), (-half + u, 2.0 * u)]
            }
            Self::Triple => {
                let u = w / 6.0;
                vec![(half - u / 2.0, u), (0.0, 2.0 * u), (-half + u / 2.0, u)]
            }
        }
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
    fn compound_lines_share_the_width() {
        assert_eq!(CompoundLine::Single.lines(6.0), [(0.0, 6.0)]);
        assert_eq!(CompoundLine::Double.lines(6.0), [(2.0, 2.0), (-2.0, 2.0)]);
        // Thick outside thin: the thick line's outer edge and the thin line's inner edge are the band's.
        assert_eq!(CompoundLine::ThickThin.lines(8.0), [(2.0, 4.0), (-3.0, 2.0)]);
        assert_eq!(CompoundLine::ThinThick.lines(8.0), [(3.0, 2.0), (-2.0, 4.0)]);
        assert_eq!(CompoundLine::Triple.lines(6.0), [(2.5, 1.0), (0.0, 2.0), (-2.5, 1.0)]);
        for c in ["sng", "dbl", "thickThin", "thinThick", "tri"] {
            assert_eq!(CompoundLine::from_ooxml(c).to_ooxml(), c);
        }
        assert!(CompoundLine::from_ooxml("bogus").is_single());
    }

    #[test]
    fn custom_dashes_are_bounded() {
        let pattern = LineDash::Custom(vec![(0, u32::MAX); MAX_DASH_STOPS + 1]).pattern(f32::NAN);
        assert_eq!(pattern.len(), MAX_DASH_STOPS * 2);
        assert!(pattern.iter().all(|v| v.is_finite() && *v >= MIN_DASH_POINTS && *v <= MAX_DASH_POINTS));
    }
}
