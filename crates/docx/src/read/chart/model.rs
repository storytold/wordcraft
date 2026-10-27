use super::*;

/// A data label to draw once the chart is: the value at (`x`, `y`), centred there (`inside`, in
/// white on the fill) or just beside it (above, or right of a horizontal bar's end).
pub(super) struct DataLabel {
    pub(super) x: f32,
    pub(super) y: f32,
    pub(super) text: String,
    pub(super) inside: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Kind {
    Bar,
    Line,
    Area,
    Pie,
    Doughnut,
    Scatter,
    Radar,
}

impl Kind {
    pub(super) fn of(name: &str) -> Option<Kind> {
        Some(match name {
            "c:barChart" => Kind::Bar,
            "c:lineChart" => Kind::Line,
            "c:areaChart" => Kind::Area,
            "c:pieChart" => Kind::Pie,
            "c:doughnutChart" => Kind::Doughnut,
            "c:scatterChart" => Kind::Scatter,
            "c:radarChart" => Kind::Radar,
            _ => return None,
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Grouping {
    Clustered,
    Stacked,
    Percent,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Pos {
    Right,
    Left,
    Top,
    Bottom,
}

/// One value axis (or category axis) as the file states it.
#[derive(Default)]
pub(super) struct Axis {
    pub(super) deleted: bool,
    pub(super) grid: bool,
    pub(super) line: bool,
    /// `c:crossBetween val="midCat"`: categories sit on the tick marks, not between them.
    pub(super) mid: bool,
    pub(super) min: Option<f64>,
    pub(super) max: Option<f64>,
    /// Number format of the tick labels, when not linked to the source data.
    pub(super) fmt: Option<String>,
}

pub(super) struct Series {
    pub(super) name: String,
    pub(super) color: Option<Rgb>,
    pub(super) line_w: f32,
    /// Whether the series is drawn as a line (scatter series without one get markers).
    pub(super) line: bool,
    pub(super) values: Vec<Option<f64>>,
    /// Scatter x values (index + 1 when the file has none).
    pub(super) xs: Vec<Option<f64>>,
    /// Per-point colours from `c:dPt`.
    pub(super) points: Vec<Option<Rgb>>,
    pub(super) fmt: Option<String>,
    /// Format code of the scatter x values' cache.
    pub(super) x_fmt: Option<String>,
    /// Data labels on or off for this series (`c:dLbls` of its own), when it says.
    pub(super) labels: Option<bool>,
    /// The values as the file has them, for data labels (`values` become shares in a 100%
    /// stacked chart); empty when the series shows no labels.
    pub(super) raw: Vec<Option<f64>>,
}
