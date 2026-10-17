//! Charts WordCraft makes and edits (Insert › Chart): the chart type, its data (categories and
//! series), title, legend and data labels.
//!
//! A chart object ([`crate::graphic::Graphic`] with `chart` set) keeps this model next to the
//! items it is drawn with; the DOCX writer turns it into a chart part (`c:chartSpace`) with the
//! data cached in the part itself, and the reader turns a chart part back into it when the part
//! holds nothing this model leaves out. Charts from other programs stay as they were read.
//!
//! Every value from outside (commands, files) goes through [`ChartSpec::sanitize`]: series,
//! points and text lengths are capped and values that aren't finite numbers become gaps.

use serde::{Deserialize, Serialize};

use crate::props::Rgb;

/// Most series in a chart.
pub const MAX_SERIES: usize = 255;
/// Most categories (points per series) in a chart.
pub const MAX_POINTS: usize = 4000;
/// Most cells (series × categories) in a chart: what the chart drawing reads at most.
pub const MAX_CELLS: usize = 100_000;
/// Longest title, series name or category label, characters.
pub const MAX_TEXT: usize = 255;

/// What a chart shows its data as.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ChartType {
    /// Clustered columns.
    #[default]
    Column,
    /// Stacked columns.
    ColumnStacked,
    /// Clustered horizontal bars.
    Bar,
    /// Stacked horizontal bars.
    BarStacked,
    Line,
    Area,
    Pie,
    Doughnut,
    /// XY scatter with markers: the categories are the x values.
    Scatter,
}

impl ChartType {
    pub const ALL: [ChartType; 9] = [
        ChartType::Column,
        ChartType::ColumnStacked,
        ChartType::Bar,
        ChartType::BarStacked,
        ChartType::Line,
        ChartType::Area,
        ChartType::Pie,
        ChartType::Doughnut,
        ChartType::Scatter,
    ];

    /// The id commands use (`column`, `barStacked`…).
    pub fn id(self) -> &'static str {
        match self {
            ChartType::Column => "column",
            ChartType::ColumnStacked => "columnStacked",
            ChartType::Bar => "bar",
            ChartType::BarStacked => "barStacked",
            ChartType::Line => "line",
            ChartType::Area => "area",
            ChartType::Pie => "pie",
            ChartType::Doughnut => "doughnut",
            ChartType::Scatter => "scatter",
        }
    }

    /// The type an id names (any ASCII case).
    pub fn from_id(id: &str) -> Option<ChartType> {
        ChartType::ALL.into_iter().find(|t| t.id().eq_ignore_ascii_case(id.trim()))
    }

    /// The name shown for it.
    pub fn label(self) -> &'static str {
        match self {
            ChartType::Column => "Clustered Column",
            ChartType::ColumnStacked => "Stacked Column",
            ChartType::Bar => "Clustered Bar",
            ChartType::BarStacked => "Stacked Bar",
            ChartType::Line => "Line",
            ChartType::Area => "Area",
            ChartType::Pie => "Pie",
            ChartType::Doughnut => "Doughnut",
            ChartType::Scatter => "Scatter",
        }
    }

    /// Pie and doughnut charts show one series, a slice per category.
    pub fn is_round(self) -> bool {
        matches!(self, ChartType::Pie | ChartType::Doughnut)
    }
}

/// Where a chart's legend sits.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum LegendPos {
    Right,
    Left,
    Top,
    #[default]
    Bottom,
}

impl LegendPos {
    pub const ALL: [LegendPos; 4] = [LegendPos::Right, LegendPos::Top, LegendPos::Left, LegendPos::Bottom];

    pub fn id(self) -> &'static str {
        match self {
            LegendPos::Right => "right",
            LegendPos::Left => "left",
            LegendPos::Top => "top",
            LegendPos::Bottom => "bottom",
        }
    }

    pub fn from_id(id: &str) -> Option<LegendPos> {
        LegendPos::ALL.into_iter().find(|p| p.id().eq_ignore_ascii_case(id.trim()))
    }
}

/// One series: its name, a value per category (`None` = no value, a gap) and how it is drawn
/// (Chart Design ▸ Format Series): fill, line, markers, smooth lines, its own data labels and
/// number format. Unset fields follow the chart's defaults.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChartSeries {
    pub name: String,
    #[serde(default)]
    pub values: Vec<Option<f64>>,
    /// Fill colour; `None`: the chart's accent colour for the series.
    #[serde(default)]
    pub fill: Option<Rgb>,
    /// "None" fill: the series draws no fill.
    #[serde(default)]
    pub no_fill: bool,
    /// Line width in points; `None`: the chart's default.
    #[serde(default)]
    pub line_w: Option<f32>,
    /// "None" line: the series draws no line.
    #[serde(default)]
    pub no_line: bool,
    /// Markers on points; `None`: the chart's default (scatter charts draw them).
    #[serde(default)]
    pub markers: Option<bool>,
    /// Draw the line as a smooth curve.
    #[serde(default)]
    pub smooth: bool,
    /// Data labels for this series; `None`: follow the chart's setting.
    #[serde(default)]
    pub labels: Option<bool>,
    /// Number format of this series's labels and values; `None`: linked to the source.
    #[serde(default)]
    pub fmt: Option<String>,
}

/// One chart axis as Word formats it (Format Axis): title, scale bounds, tick-label number
/// format, gridlines, tick labels and the axis line.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChartAxis {
    /// `None`: no axis title; text cut to [`MAX_TEXT`] characters.
    #[serde(default)]
    pub title: Option<String>,
    /// Fixed lower bound; `None`: automatic.
    #[serde(default)]
    pub min: Option<f64>,
    /// Fixed upper bound; `None`: automatic.
    #[serde(default)]
    pub max: Option<f64>,
    /// Tick-label number format (`#,##0`, `0.00`, `0%`…); `None`: linked to the source data.
    #[serde(default)]
    pub fmt: Option<String>,
    /// Major gridlines drawn along this axis.
    #[serde(default)]
    pub gridlines: bool,
    /// Tick labels; `false` hides the axis the way Word's Axis Labels ▸ None does.
    #[serde(default = "the_true")]
    pub labels: bool,
    /// The axis line.
    #[serde(default = "the_true")]
    pub line: bool,
}

fn the_true() -> bool {
    true
}

impl Default for ChartAxis {
    fn default() -> Self {
        ChartAxis { title: None, min: None, max: None, fmt: None, gridlines: false, labels: true, line: true }
    }
}

/// The value axis as a new chart has it: gridlines, labels, no axis line.
pub fn value_axis() -> ChartAxis {
    ChartAxis { gridlines: true, line: false, ..ChartAxis::default() }
}

impl ChartAxis {
    /// Make the axis safe to store: texts cleaned, bounds finite, `min` under `max` (an
    /// impossible pair drops both), the format code cut to [`MAX_FMT`] characters.
    pub fn sanitize(&mut self) {
        self.title = self.title.as_deref().map(clean_text).filter(|t| !t.is_empty());
        self.min = self.min.filter(|v| v.is_finite());
        self.max = self.max.filter(|v| v.is_finite());
        if matches!((self.min, self.max), (Some(lo), Some(hi)) if lo >= hi) {
            self.min = None;
            self.max = None;
        }
        self.fmt = self.fmt.as_deref().map(clean_fmt).filter(|f| !f.is_empty());
    }
}

/// Longest axis number format kept, characters.
pub const MAX_FMT: usize = 128;

/// A number format code: control characters as spaces, ends trimmed, at most [`MAX_FMT`]
/// characters.
pub fn clean_fmt(s: &str) -> String {
    let s: String = s.chars().take(MAX_FMT * 4).map(|c| if c.is_control() { ' ' } else { c }).collect();
    s.trim().chars().take(MAX_FMT).collect::<String>().trim_end().to_string()
}

/// A chart WordCraft can edit.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChartSpec {
    #[serde(rename = "type")]
    pub kind: ChartType,
    /// Category labels (scatter charts: the x values).
    #[serde(default)]
    pub categories: Vec<String>,
    #[serde(default)]
    pub series: Vec<ChartSeries>,
    #[serde(default)]
    pub title: Option<String>,
    /// `None`: no legend.
    #[serde(default)]
    pub legend: Option<LegendPos>,
    /// Show each point's value next to it.
    #[serde(default)]
    pub data_labels: bool,
    /// The category axis (scatter charts: the x axis).
    #[serde(default)]
    pub cat_axis: ChartAxis,
    /// The value axis (scatter charts: the y axis); gridlines on, no axis line, as a new chart has them.
    #[serde(default = "value_axis")]
    pub val_axis: ChartAxis,
}

impl ChartSpec {
    /// A new chart of type `kind` with WordCraft's sample data: four quarters and three series
    /// (one for pie and doughnut charts, x values for scatter charts), legend at the bottom.
    pub fn sample(kind: ChartType) -> ChartSpec {
        let (categories, series): (Vec<&str>, Vec<(&str, [f64; 4])>) = match kind {
            ChartType::Pie | ChartType::Doughnut => (vec!["North", "East", "South", "West"], vec![("Share", [38.0, 24.0, 21.0, 17.0])]),
            ChartType::Scatter => (vec!["1", "2", "3", "4"], vec![("Series 1", [2.1, 3.4, 3.0, 4.6]), ("Series 2", [1.2, 1.9, 2.8, 3.1])]),
            _ => (
                vec!["Q1", "Q2", "Q3", "Q4"],
                vec![("Series 1", [3.2, 4.1, 3.6, 5.0]), ("Series 2", [2.0, 2.7, 3.9, 3.3]), ("Series 3", [1.5, 2.2, 2.4, 3.8])],
            ),
        };
        ChartSpec {
            kind,
            categories: categories.into_iter().map(str::to_string).collect(),
            series: series
                .into_iter()
                .map(|(n, v)| ChartSeries {
                    name: n.to_string(),
                    values: v.into_iter().map(Some).collect(),
                    // Scatter series are drawn as markers without a line.
                    no_line: kind == ChartType::Scatter,
                    ..Default::default()
                })
                .collect(),
            title: None,
            legend: Some(LegendPos::Bottom),
            data_labels: false,
            cat_axis: ChartAxis::default(),
            // Pie and doughnut charts have no axes: their axes stay plain, so the chart reads back
            // as the same model.
            val_axis: if kind.is_round() { ChartAxis::default() } else { value_axis() },
        }
    }

    /// Number of categories: every series has as many values.
    pub fn points(&self) -> usize {
        self.categories.len()
    }

    /// Make the chart safe to store and draw: text cut to [`MAX_TEXT`] characters with control
    /// characters turned into spaces and the ends trimmed (an empty title is none); at most
    /// [`MAX_POINTS`] categories, [`MAX_SERIES`] series and [`MAX_CELLS`] cells; values that
    /// aren't finite become gaps; every series as long as the categories (padded with gaps,
    /// categories padded with empty labels to the longest series).
    pub fn sanitize(&mut self) {
        self.series.truncate(MAX_SERIES);
        let longest = self.series.iter().map(|s| s.values.len()).max().unwrap_or(0);
        let n = self.categories.len().max(longest).min(MAX_POINTS);
        let keep = MAX_CELLS.checked_div(n).unwrap_or(MAX_SERIES).clamp(1, MAX_SERIES);
        self.series.truncate(keep);
        self.categories.truncate(n);
        for c in &mut self.categories {
            *c = clean_text(c);
        }
        self.categories.resize(n, String::new());
        for s in &mut self.series {
            s.name = clean_text(&s.name);
            s.values.truncate(n);
            for v in &mut s.values {
                *v = v.filter(|x| x.is_finite());
            }
            s.values.resize(n, None);
            s.fmt = s.fmt.as_deref().map(clean_fmt).filter(|f| !f.is_empty());
            s.line_w = s.line_w.filter(|w| w.is_finite()).map(|w| w.clamp(0.0, 20.0));
        }
        self.title = self.title.as_deref().map(clean_text).filter(|t| !t.is_empty());
        self.cat_axis.sanitize();
        self.val_axis.sanitize();
    }

    /// The chart after `sanitize`.
    pub fn sanitized(mut self) -> ChartSpec {
        self.sanitize();
        self
    }
}

/// `s` as chart text: control characters as spaces, ends trimmed, at most [`MAX_TEXT`] characters.
pub fn clean_text(s: &str) -> String {
    let s: String = s.chars().take(MAX_TEXT * 4).map(|c| if c.is_control() { ' ' } else { c }).collect();
    s.trim().chars().take(MAX_TEXT).collect::<String>().trim_end().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitize_caps_sizes_and_drops_non_finite_values() {
        let mut c = ChartSpec {
            categories: vec!["a".into(); MAX_POINTS + 10],
            series: (0..MAX_SERIES + 5)
                .map(|i| ChartSeries {
                    name: format!("{}\n{}", "x".repeat(400), i),
                    values: vec![Some(f64::NAN), Some(f64::INFINITY), Some(1.0)],
                    ..Default::default()
                })
                .collect(),
            title: Some("  ".into()),
            ..Default::default()
        };
        c.sanitize();
        assert_eq!(c.points(), MAX_POINTS);
        assert!(c.series.len() * c.points() <= MAX_CELLS);
        assert!(!c.series.is_empty());
        let s = &c.series[0];
        assert_eq!(s.values.len(), MAX_POINTS);
        assert_eq!(&s.values[..3], &[None, None, Some(1.0)]);
        assert_eq!(s.name.chars().count(), MAX_TEXT);
        assert!(!s.name.contains('\n'));
        assert_eq!(c.title, None);
    }

    #[test]
    fn short_categories_are_padded_to_the_longest_series() {
        let c = ChartSpec {
            categories: vec!["a".into()],
            series: vec![ChartSeries { name: "s".into(), values: vec![Some(1.0), Some(2.0)], ..Default::default() }],
            ..Default::default()
        }
        .sanitized();
        assert_eq!(c.categories, vec!["a".to_string(), String::new()]);
        assert_eq!(ChartType::from_id("BARSTACKED"), Some(ChartType::BarStacked));
        assert_eq!(LegendPos::from_id("top"), Some(LegendPos::Top));
    }

    #[test]
    fn axis_bounds_and_formats_are_cleaned() {
        let mut a = ChartAxis {
            title: Some(" Sales\n".into()),
            min: Some(f64::NAN),
            max: Some(50.0),
            fmt: Some("x".repeat(MAX_FMT + 40)),
            gridlines: true,
            labels: false,
            line: true,
        };
        a.sanitize();
        assert_eq!(a.title.as_deref(), Some("Sales"));
        assert_eq!(a.min, None);
        assert_eq!(a.max, Some(50.0));
        assert_eq!(a.fmt.as_deref().map(|f| f.chars().count()), Some(MAX_FMT));

        // min not under max drops the pair; a format of only spaces is no format.
        let mut b = ChartAxis { min: Some(50.0), max: Some(50.0), fmt: Some("  ".into()), ..Default::default() };
        b.sanitize();
        assert_eq!((b.min, b.max), (None, None));
        assert_eq!(b.fmt, None);

        let mut spec = ChartSpec {
            series: vec![ChartSeries { name: "s".into(), values: vec![Some(1.0)], ..Default::default() }],
            categories: vec!["a".into()],
            ..Default::default()
        };
        spec.series[0].line_w = Some(f32::NAN);
        spec.series[0].fmt = Some("0.00".into());
        spec.sanitize();
        assert_eq!(spec.series[0].line_w, None);
        assert_eq!(spec.series[0].fmt.as_deref(), Some("0.00"));
        let mut spec = ChartSpec {
            series: vec![ChartSeries { name: "s".into(), values: vec![], line_w: Some(100.0), ..Default::default() }],
            ..Default::default()
        };
        spec.sanitize();
        assert_eq!(spec.series[0].line_w, Some(20.0), "a wild width clamps to 20 pt");
    }
}
