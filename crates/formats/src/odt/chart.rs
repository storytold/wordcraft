//! Charts embedded in a document (ODF 1.3 §10.4.3 `draw:object` whose sub-document is an
//! `office:chart`, Part 3 §11): read into WordCraft's editable chart model when the chart is one
//! WordCraft can edit as it is (one plot type in two dimensions, its data in the chart's own
//! table). Anything else returns `None`, and the frame keeps its replacement picture.
//!
//! The data comes from the chart's `table:table` (§11.1): the series' and categories' cell range
//! addresses (§19.583, `local-table.$B$2:.$B$5`) point into it, counting its header rows and
//! columns like any other.

use std::collections::HashMap;

use wordcraft_doc::chart::{ChartSeries, ChartSpec, ChartType, LegendPos, MAX_CELLS, MAX_POINTS, MAX_SERIES, MAX_TEXT};

use super::xml::Attrs;
use super::{Ev, events, get};

/// Most XML events read from a chart sub-document.
const MAX_EVENTS: usize = 2_000_000;
/// Most rows read from a chart's table: a header row and a row per category.
const MAX_ROWS: usize = MAX_POINTS + 1;
/// Most columns read from it: a label column and two per series (a scatter series' x values).
const MAX_COLS: usize = MAX_SERIES * 2 + 1;

/// One cell of the chart's table: its text, and its number when it holds one (`Some(None)`: a
/// number cell without a usable value, LibreOffice's `NaN` gap).
#[derive(Clone, Default)]
struct DataCell {
    text: String,
    num: Option<Option<f64>>,
}

/// A `chart:series` element as read.
#[derive(Default)]
struct RawSeries {
    values: Option<String>,
    label: Option<String>,
    class: Option<String>,
    style: Option<String>,
    domain: Option<String>,
    secondary: bool,
}

#[derive(Default)]
struct Reader {
    /// Chart styles' `style:chart-properties`, by style name.
    props: HashMap<String, Attrs>,
    style: Option<String>,
    class: Option<String>,
    plot_style: Option<String>,
    title: Option<String>,
    in_title: bool,
    /// The legend's position, when there is a legend.
    legend: Option<String>,
    categories: Option<String>,
    series: Vec<RawSeries>,
    in_series: bool,
    /// The chart's table: its rows of cells, and how deep in tables the reader is.
    rows: Vec<Vec<DataCell>>,
    table_depth: usize,
    /// How many times the open row and cell repeat.
    row_repeat: usize,
    cell_repeat: usize,
    cell: Option<DataCell>,
    cells: usize,
    in_chart: bool,
}

/// A count attribute (`table:number-rows-repeated`…): 1 when absent or unusable, at most `max`.
fn repeat(a: &Attrs, k: &str, max: usize) -> usize {
    get(a, k).and_then(|v| v.trim().parse::<usize>().ok()).unwrap_or(1).clamp(1, max.max(1))
}

impl Reader {
    fn start(&mut self, q: &str, a: &Attrs) {
        match q {
            "style:style" => self.style = get(a, "style:name").map(str::to_string),
            "style:chart-properties" => {
                if let Some(name) = self.style.clone() {
                    self.props.insert(name, a.clone());
                }
            }
            "chart:chart" if !self.in_chart => {
                self.in_chart = true;
                self.class = get(a, "chart:class").map(str::to_string);
            }
            "chart:title" => self.in_title = true,
            "text:p" if self.in_title => {
                let t = self.title.get_or_insert_with(String::new);
                if !t.is_empty() {
                    t.push(' ');
                }
            }
            "chart:legend" => self.legend = Some(get(a, "chart:legend-position").unwrap_or("end").to_string()),
            "chart:plot-area" => self.plot_style = get(a, "chart:style-name").map(str::to_string),
            "chart:categories" if self.categories.is_none() => self.categories = get(a, "table:cell-range-address").map(str::to_string),
            "chart:series" => {
                self.in_series = true;
                if self.series.len() < MAX_SERIES {
                    self.series.push(RawSeries {
                        values: get(a, "chart:values-cell-range-address").map(str::to_string),
                        label: get(a, "chart:label-cell-address").map(str::to_string),
                        class: get(a, "chart:class").map(str::to_string),
                        style: get(a, "chart:style-name").map(str::to_string),
                        domain: None,
                        secondary: get(a, "chart:attached-axis").is_some_and(|v| v.starts_with("secondary")),
                    });
                }
            }
            "chart:domain" if self.in_series => {
                if let Some(s) = self.series.last_mut().filter(|s| s.domain.is_none()) {
                    s.domain = get(a, "table:cell-range-address").map(str::to_string);
                }
            }
            "table:table" => self.table_depth += 1,
            "table:table-row" if self.table_depth == 1 => {
                self.row_repeat = repeat(a, "table:number-rows-repeated", MAX_ROWS);
                if self.rows.len() < MAX_ROWS {
                    self.rows.push(Vec::new());
                }
            }
            "table:table-cell" | "table:covered-table-cell" if self.table_depth == 1 => {
                self.cell_repeat = repeat(a, "table:number-columns-repeated", MAX_COLS);
                let num = (get(a, "office:value-type") == Some("float") || get(a, "office:value-type") == Some("percentage"))
                    .then(|| get(a, "office:value").and_then(|v| v.trim().parse::<f64>().ok()).filter(|v| v.is_finite()));
                self.cell = Some(DataCell { text: String::new(), num });
            }
            _ => {}
        }
    }

    fn end(&mut self, q: &str) {
        match q {
            "style:style" => self.style = None,
            "chart:title" => self.in_title = false,
            "chart:series" => self.in_series = false,
            "table:table" => self.table_depth = self.table_depth.saturating_sub(1),
            "table:table-cell" | "table:covered-table-cell" if self.table_depth == 1 => {
                if let Some(cell) = self.cell.take()
                    && let Some(row) = self.rows.last_mut()
                {
                    let n = self.cell_repeat.min(MAX_COLS.saturating_sub(row.len())).min(MAX_CELLS.saturating_sub(self.cells));
                    self.cells += n;
                    row.extend(std::iter::repeat_n(cell, n));
                }
            }
            "table:table-row" if self.table_depth == 1 => {
                // A repeated row: its copies, within the table's and the chart's limits.
                if let Some(row) = self.rows.last().cloned() {
                    let room = MAX_ROWS.saturating_sub(self.rows.len());
                    let by_cells = MAX_CELLS.saturating_sub(self.cells).checked_div(row.len()).unwrap_or(room);
                    let n = self.row_repeat.saturating_sub(1).min(room).min(by_cells);
                    self.cells += n.saturating_mul(row.len());
                    self.rows.extend(std::iter::repeat_n(row, n));
                }
                self.row_repeat = 1;
            }
            _ => {}
        }
    }

    fn text(&mut self, t: &str) {
        if self.in_title
            && let Some(title) = &mut self.title
            && title.len() < MAX_TEXT * 4
        {
            title.push_str(t);
        } else if let Some(cell) = &mut self.cell
            && cell.text.len() < MAX_TEXT * 4
        {
            cell.text.push_str(t);
        }
    }

    /// The chart properties of style `name`, attribute `k`.
    fn prop(&self, name: Option<&str>, k: &str) -> Option<&str> {
        name.and_then(|n| self.props.get(n)).and_then(|a| get(a, k))
    }

    fn cell(&self, (col, row): (usize, usize)) -> Option<&DataCell> {
        self.rows.get(row).and_then(|r| r.get(col))
    }

    /// The cells a range address covers, in order, at most [`MAX_POINTS`].
    fn range(&self, addr: &str) -> Vec<Option<&DataCell>> {
        cells_of(addr).into_iter().map(|at| self.cell(at)).collect()
    }

    fn chart(&self) -> Option<ChartSpec> {
        let class = self.class.as_deref().map(|c| c.trim_start_matches("chart:"))?;
        let plot = self.plot_style.as_deref();
        let on = |k: &str| self.prop(plot, k) == Some("true");
        if on("chart:three-dimensional") || on("chart:percentage") || on("chart:deep") {
            return None;
        }
        let stacked = on("chart:stacked");
        let kind = match class {
            "bar" => match (on("chart:vertical"), stacked) {
                (false, false) => ChartType::Column,
                (false, true) => ChartType::ColumnStacked,
                (true, false) => ChartType::Bar,
                (true, true) => ChartType::BarStacked,
            },
            "line" if !stacked => ChartType::Line,
            "area" if !stacked => ChartType::Area,
            "circle" => ChartType::Pie,
            "ring" => ChartType::Doughnut,
            "scatter" => ChartType::Scatter,
            _ => return None,
        };
        // One plot type on one value axis: a series of another type, or on a second axis, is
        // a combination chart WordCraft can't edit.
        let own = |c: &str| {
            let c = c.trim_start_matches("chart:");
            c == class || (kind == ChartType::Doughnut && c == "circle")
        };
        if self.series.is_empty() || self.series.iter().any(|s| s.secondary || !s.class.as_deref().is_none_or(own)) {
            return None;
        }
        let label = |c: Option<&DataCell>| c.map(|c| c.text.trim().to_string()).unwrap_or_default();
        let series = self
            .series
            .iter()
            .enumerate()
            .map(|(i, s)| {
                // A series without its own range takes the table's next column.
                let values = match &s.values {
                    Some(addr) => self.range(addr).into_iter().map(|c| c.and_then(value)).collect(),
                    None => self.rows.iter().skip(1).take(MAX_POINTS).map(|r| r.get(i + 1).and_then(value)).collect(),
                };
                let name = s.label.as_deref().and_then(|a| cells_of(a).first().copied()).map(|at| label(self.cell(at))).unwrap_or_default();
                let name = if name.is_empty() { format!("Series {}", i + 1) } else { name };
                ChartSeries { name, values }
            })
            .collect();
        // A scatter chart's x values: its first series' domain.
        let domain = self.series.first().and_then(|s| s.domain.as_deref()).filter(|_| kind == ChartType::Scatter);
        let categories = match domain.or(self.categories.as_deref()) {
            Some(addr) => self.range(addr).into_iter().map(label).collect(),
            None => Vec::new(),
        };
        let data_labels = std::iter::once(plot)
            .chain(self.series.iter().map(|s| s.style.as_deref()))
            .any(|st| matches!(self.prop(st, "chart:data-label-number"), Some("value" | "percentage" | "value-and-percentage")));
        let legend = self.legend.as_deref().map(|p| match p {
            "start" => LegendPos::Left,
            p if p.starts_with("top") => LegendPos::Top,
            p if p.starts_with("bottom") => LegendPos::Bottom,
            _ => LegendPos::Right,
        });
        let title = self.title.as_deref().map(str::trim).filter(|t| !t.is_empty()).map(str::to_string);
        Some(ChartSpec { kind, categories, series, title, legend, data_labels }.sanitized())
    }
}

/// A cell's number: its value, or its text read as a number.
fn value(c: &DataCell) -> Option<f64> {
    match c.num {
        Some(v) => v,
        None => c.text.trim().parse::<f64>().ok().filter(|v| v.is_finite()),
    }
}

/// A cell address (`$B$2`, `local-table.B2`, `'my table'.$B$2`): its column and row, from 0.
fn cell_at(s: &str) -> Option<(usize, usize)> {
    // The table name, when there is one, ends at the last dot.
    let s = s.rsplit('.').next().unwrap_or(s).trim().trim_start_matches('$');
    let letters = s.bytes().take_while(u8::is_ascii_alphabetic).count();
    if letters == 0 || letters > 3 {
        return None;
    }
    let (col, rest) = s.split_at_checked(letters)?;
    let col = col.bytes().try_fold(0usize, |n, b| n.checked_mul(26)?.checked_add(usize::from(b.to_ascii_uppercase() - b'A') + 1))?;
    let row: usize = rest.trim_start_matches('$').parse().ok()?;
    Some((col.checked_sub(1)?, row.checked_sub(1)?))
}

/// The cells of a range address's first range (`local-table.$B$2:.$B$5`), in reading order: down
/// a column, or along a row (a block: down its first column), at most [`MAX_POINTS`].
fn cells_of(addr: &str) -> Vec<(usize, usize)> {
    let first = addr.split_whitespace().next().unwrap_or_default();
    let mut ends = first.splitn(2, ':');
    let Some(a) = ends.next().and_then(cell_at) else { return Vec::new() };
    let b = match ends.next() {
        Some(e) => match cell_at(e) {
            Some(b) => b,
            None => return Vec::new(),
        },
        None => a,
    };
    let (c0, c1) = (a.0.min(b.0), a.0.max(b.0));
    let (r0, r1) = (a.1.min(b.1), a.1.max(b.1));
    if r0 == r1 && c0 != c1 { (c0..=c1).take(MAX_POINTS).map(|c| (c, r0)).collect() } else { (r0..=r1).take(MAX_POINTS).map(|r| (c0, r)).collect() }
}

/// The chart a chart sub-document's `content.xml` holds, when WordCraft can edit it.
pub(in crate::odt) fn read_chart(xml: &[u8]) -> Option<ChartSpec> {
    let mut r = Reader::default();
    let mut count = 0usize;
    let mut done = false;
    events(xml, |ev| {
        count += 1;
        if done || count > MAX_EVENTS {
            return;
        }
        match ev {
            Ev::Start(q, a) => r.start(&q, &a),
            Ev::End(q) => {
                if q == "chart:chart" && r.in_chart {
                    done = true;
                }
                r.end(&q);
            }
            Ev::Text(t) => r.text(&t),
        }
    });
    // A sub-document cut off before its chart ended is no chart.
    if !done {
        return None;
    }
    r.chart()
}
