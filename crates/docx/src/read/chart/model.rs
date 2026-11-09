use super::Rgb;

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
    Stock,
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
            "c:stockChart" => Kind::Stock,
            _ => return None,
        })
    }

    /// Drawing order in a plot area of several chart types: areas at the back, then bars, then
    /// lines (stock included), scatters in front.
    pub(super) fn layer(self) -> u8 {
        match self {
            Kind::Area => 0,
            Kind::Bar => 1,
            Kind::Line | Kind::Stock => 2,
            Kind::Scatter => 3,
            Kind::Pie | Kind::Doughnut | Kind::Radar => 4,
        }
    }
}

/// One chart type element of the plot area (`c:barChart`, `c:lineChart`…) and its series.
pub(super) struct Group {
    pub(super) kind: Kind,
    pub(super) grouping: Grouping,
    /// Gap between groups as a fraction of a bar's width.
    pub(super) gap: f32,
    /// Overlap of clustered bars, -1..1 (1 = all on top of each other).
    pub(super) overlap: f32,
    pub(super) series: Vec<Series>,
    /// Plotted on the secondary value axis.
    pub(super) secondary: bool,
    /// Position of the group's first series among all the chart's series.
    pub(super) first: usize,
    /// Hi-low lines and up/down bars (stock and line charts).
    pub(super) stock: Stock,
}

impl Group {
    pub(super) fn stacked(&self) -> bool {
        self.grouping != Grouping::Clustered
    }
}

/// Hi-low lines (`c:hiLowLines`) and up/down bars (`c:upDownBars`) of a stock or line chart.
#[derive(Default)]
pub(super) struct Stock {
    /// Colour and width of the hi-low lines, when drawn.
    pub(super) hi_low: Option<(Rgb, f32)>,
    pub(super) up_down: Option<UpDown>,
}

/// Up/down bars: their gap (fraction of a bar's width) and the style of rising and falling ones.
pub(super) struct UpDown {
    pub(super) gap: f32,
    pub(super) up: BarStyle,
    pub(super) down: BarStyle,
}

pub(super) struct BarStyle {
    pub(super) fill: Option<Rgb>,
    pub(super) outline: Option<(Rgb, f32)>,
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
    /// The series' `c:idx` (an `xsd:unsignedInt`), which picks its automatic colour; `None`
    /// when missing or not a valid number.
    pub(super) idx: Option<usize>,
    /// `c:order`: the place in the legend.
    pub(super) order: Option<i64>,
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
