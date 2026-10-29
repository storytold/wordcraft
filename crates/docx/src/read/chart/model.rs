use super::{Fill, Rgb};

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
    /// Colour of the tick labels, when the axis's `c:txPr` states one.
    pub(super) text: Option<Rgb>,
    /// The display unit the tick labels count in (`c:dispUnits`): 1000 for thousands.
    pub(super) unit: f64,
    /// The display unit's label, when the axis shows one: its text, colour and weight.
    pub(super) unit_label: Option<(String, Option<Rgb>, bool)>,
    /// Where this axis crosses the other one (`c:crosses` / `c:crossesAt`).
    pub(super) crosses: Crosses,
    /// Where its tick labels sit (`c:tickLblPos`).
    pub(super) tick_pos: TickPos,
    /// The axis line's colour and width, when its `c:spPr` states them.
    pub(super) line_colour: Option<Rgb>,
    pub(super) line_w: f32,
    /// Bold tick labels, when its text properties say.
    pub(super) bold: Option<bool>,
}

impl Default for Axis {
    fn default() -> Self {
        Axis {
            deleted: false,
            grid: false,
            line: false,
            mid: false,
            min: None,
            max: None,
            fmt: None,
            text: None,
            unit: 1.0,
            unit_label: None,
            crosses: Crosses::AutoZero,
            tick_pos: TickPos::NextTo,
            line_colour: None,
            line_w: 0.75,
            bold: None,
        }
    }
}

/// Where an axis crosses the other axis.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) enum Crosses {
    /// At zero, or at the end of the range nearer zero when zero is outside it.
    AutoZero,
    Min,
    Max,
    At(f64),
}

/// Where an axis's tick labels sit.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum TickPos {
    /// Beside the axis, wherever it crosses.
    NextTo,
    /// At the low end of the other axis (under a column chart's plot).
    Low,
    /// At the high end.
    High,
    None,
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
    /// The values as the file has them, for data labels and a data table (`values` become
    /// shares in a 100% stacked chart); empty when neither shows them.
    pub(super) raw: Vec<Option<f64>>,
    /// The chart type the series is drawn as: a combination chart has more than one.
    pub(super) kind: Kind,
    /// The marker drawn at each point of a line series, when it shows them.
    pub(super) marker: Option<Marker>,
    /// A gradient fill (`a:gradFill`) its columns are painted with; `color` is its first colour.
    pub(super) shading: Option<Fill>,
}

/// A line series' point marker.
#[derive(Clone, Copy, Debug)]
pub(super) struct Marker {
    pub(super) shape: MarkerShape,
    /// Width and height, points.
    pub(super) size: f32,
    pub(super) fill: Option<Rgb>,
    pub(super) line: Option<Rgb>,
}

/// The marker shapes drawn; the others (x, star, plus, dash, dot, picture) are drawn as squares.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum MarkerShape {
    Square,
    Diamond,
    Triangle,
    Circle,
}

impl MarkerShape {
    /// The shape a `c:symbol` names; `None` for `none`. `auto` is the series' own, [`MarkerShape::auto`].
    pub(super) fn of(symbol: &str) -> Option<MarkerShape> {
        Some(match symbol {
            "none" => return None,
            "diamond" => MarkerShape::Diamond,
            "triangle" => MarkerShape::Triangle,
            "circle" => MarkerShape::Circle,
            _ => MarkerShape::Square,
        })
    }

    /// The automatic marker of the series with index `idx`: diamond, square, triangle, x, star,
    /// circle, plus, in turn (the ones not drawn as such fall back to squares).
    pub(super) fn auto(idx: usize) -> MarkerShape {
        match idx % 7 {
            0 => MarkerShape::Diamond,
            2 => MarkerShape::Triangle,
            5 => MarkerShape::Circle,
            _ => MarkerShape::Square,
        }
    }
}

/// How a legend entry (and a data table row) shows its series: a colour swatch, or for a line
/// series a stretch of line with its marker.
#[derive(Clone, Copy, Debug)]
pub(super) enum Key {
    Swatch,
    Line(Option<Marker>),
}

/// A chart's data table (`c:dTable`): the plotted values in a grid under the plot.
pub(super) struct DataTable {
    pub(super) horz: bool,
    pub(super) vert: bool,
    pub(super) outline: bool,
    pub(super) keys: bool,
    pub(super) text: Rgb,
    pub(super) bold: bool,
    pub(super) line: Rgb,
}
