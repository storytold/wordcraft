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

/// One series: its name and a value per category (`None` = no value, a gap).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChartSeries {
    pub name: String,
    #[serde(default)]
    pub values: Vec<Option<f64>>,
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
            series: series.into_iter().map(|(n, v)| ChartSeries { name: n.to_string(), values: v.into_iter().map(Some).collect() }).collect(),
            title: None,
            legend: Some(LegendPos::Bottom),
            data_labels: false,
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
        }
        self.title = self.title.as_deref().map(clean_text).filter(|t| !t.is_empty());
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
                .map(|i| ChartSeries { name: format!("{}\n{}", "x".repeat(400), i), values: vec![Some(f64::NAN), Some(f64::INFINITY), Some(1.0)] })
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
            series: vec![ChartSeries { name: "s".into(), values: vec![Some(1.0), Some(2.0)] }],
            ..Default::default()
        }
        .sanitized();
        assert_eq!(c.categories, vec!["a".to_string(), String::new()]);
        assert_eq!(ChartType::from_id("BARSTACKED"), Some(ChartType::BarStacked));
        assert_eq!(LegendPos::from_id("top"), Some(LegendPos::Top));
    }
}
