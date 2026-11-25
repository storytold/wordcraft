//! Editing charts made by other programs (Word, Excel, LibreOffice…) without losing their look.
//!
//! A chart read from a file that WordCraft's own model can't hold ([`wordcraft_doc::chart`]) is
//! kept as it was read: its `a:graphic` on the object and its chart part, embedded workbook and
//! style parts in `Document::passthrough` (see `craft_docx`'s embedded-part manifest). Until now
//! such a chart could only be shown. This module edits it in place instead of converting it:
//!
//! - [`read_part`] reads what Chart Design shows from a chart part (`c:chartSpace`, ECMA-376
//!   Part 1 §21.2): the categories, the series names and values, the title, the legend and the
//!   data labels, through every chart type group of the plot area (combo charts included).
//! - [`edit_part`] changes those in the part's markup and leaves everything else as it was read:
//!   series formatting, axes, number formats, chart and colour styles, extension lists. The new
//!   data goes into the caches (`c:strCache`, `c:numCache`) and the cell references (`c:f`) are
//!   written in the layout Word gives a chart's sheet: categories down column A from row 2,
//!   series names along row 1 from column B, each series' values under its name.
//! - [`edit_workbook`] writes the same cells into the chart's embedded workbook, so Word's own
//!   Edit Data, which reads the sheet and refreshes the chart from it, shows the new data.
//! - [`draw_part`] draws a chart part with the reader's own chart drawing.
//!
//! Every value from outside goes through the same limits as the model ([`MAX_SERIES`],
//! [`MAX_POINTS`], [`MAX_TEXT`]).

use craft_xml::{El, Local, Node};
use wordcraft_doc::chart::{ChartSeries, ChartSpec, ChartType, LegendPos, MAX_POINTS, MAX_SERIES, clean_text};

mod draw;
mod kept;
mod workbook;

#[cfg(test)]
mod tests;

pub use draw::draw_part;
pub use kept::{KeptEdit, MANIFEST, chart_part, edit_kept, read_kept};
pub use workbook::edit_workbook;

/// Largest chart part read, bytes.
pub const MAX_PART: usize = 16 << 20;
/// Most chart type groups read from one plot area.
const MAX_GROUPS: usize = 16;

/// A chart's data: category labels (scatter charts: the x values) and its series.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ChartData {
    pub categories: Vec<String>,
    pub series: Vec<SeriesData>,
}

/// One series: its name and a value per category (`None`: no value).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SeriesData {
    pub name: String,
    pub values: Vec<Option<f64>>,
}

impl ChartData {
    /// The number of rows the data takes: the categories or the longest series.
    pub fn points(&self) -> usize {
        self.series.iter().map(|s| s.values.len()).max().unwrap_or(0).max(self.categories.len())
    }

    /// Held to the model's limits, texts cleaned.
    pub fn bounded(&self) -> ChartData {
        ChartData {
            categories: self.categories.iter().take(MAX_POINTS).map(|c| clean_text(c)).collect(),
            series: self
                .series
                .iter()
                .take(MAX_SERIES)
                .map(|s| SeriesData {
                    name: clean_text(&s.name),
                    values: s.values.iter().take(MAX_POINTS).map(|v| v.filter(|x| x.is_finite())).collect(),
                })
                .collect(),
        }
    }
}

/// What Chart Design shows of a chart part.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PartChart {
    /// The model type closest to the part's first chart type group (for the type gallery's
    /// mark); the part keeps its own.
    pub kind: ChartType,
    pub data: ChartData,
    pub title: Option<String>,
    pub legend: Option<LegendPos>,
    pub data_labels: bool,
    /// The sheet the cell references name (`Sheet1`, `'My data'`), as written before the `!`.
    pub sheet: Option<String>,
    /// The `c:externalData` relationship id: the chart's workbook.
    pub workbook: Option<String>,
}

impl PartChart {
    /// The chart as WordCraft's model shows it (Chart Design, Edit Data): its data, title, legend
    /// and data labels; the look stays in the part.
    pub fn view(&self) -> ChartSpec {
        ChartSpec {
            kind: self.kind,
            categories: self.data.categories.clone(),
            series: self.data.series.iter().map(|s| ChartSeries::new(self.kind, s.name.clone(), s.values.clone())).collect(),
            title: self.title.clone(),
            legend: self.legend,
            data_labels: self.data_labels,
            ..ChartSpec::default()
        }
    }
}

/// A change to a chart part.
#[derive(Clone, Debug, PartialEq)]
pub enum PartEdit {
    /// New categories and series (each `None`: keep them). Series match by their place in
    /// `c:order`; added ones copy the last one's markup without its own colours, removed ones go.
    Data { categories: Option<Vec<String>>, series: Option<Vec<SeriesData>> },
    /// The title's text; `None` removes the title.
    Title(Option<String>),
    /// The legend's place; `None` removes the legend.
    Legend(Option<LegendPos>),
    /// Show or hide the values next to the points, for every series.
    DataLabels(bool),
}

/// Why a chart part can't be changed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EditError(pub String);

impl std::fmt::Display for EditError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for EditError {}

fn err<T>(msg: &str) -> Result<T, EditError> {
    Err(EditError(msg.to_string()))
}

/// Parse a chart part. `None` when it isn't one (or is too big).
fn parse(bytes: &[u8]) -> Option<El> {
    if bytes.len() > MAX_PART {
        return None;
    }
    let root = craft_xml::parse(bytes).ok()?;
    (root.local() == "chartSpace").then_some(root)
}

/// What Chart Design shows of the chart part `bytes`. `None` when it isn't a chart part, or its
/// plot area has no series.
pub fn read_part(bytes: &[u8]) -> Option<PartChart> {
    let root = parse(bytes)?;
    let chart = root.child(Local("chart"))?;
    let plot = chart.child(Local("plotArea"))?;
    let groups: Vec<&El> = groups(plot).collect();
    let first = groups.first()?;
    let scatter = first.local() == "scatterChart" || first.local() == "bubbleChart";
    let sers = ordered(&groups);
    if sers.is_empty() {
        return None;
    }
    let cat_name = if scatter { "xVal" } else { "cat" };
    let categories = sers.iter().find_map(|s| s.child(Local(cat_name))).map(cell_texts).unwrap_or_default();
    let series = sers
        .iter()
        .map(|s| SeriesData { name: series_name(s), values: s.child(Local(if scatter { "yVal" } else { "val" })).map(numbers).unwrap_or_default() })
        .collect();
    let data = ChartData { categories, series }.bounded();
    let title = title_text(chart, &data);
    let legend = chart.child(Local("legend")).map(|l| match l.child(Local("legendPos")).and_then(|p| p.attr("val")) {
        Some("l") => LegendPos::Left,
        Some("t") => LegendPos::Top,
        Some("b") => LegendPos::Bottom,
        _ => LegendPos::Right,
    });
    let data_labels = groups.iter().any(|g| shows_values(g.child(Local("dLbls")))) || sers.iter().all(|s| shows_values(s.child(Local("dLbls"))));
    let sheet = root.find(Local("f")).and_then(|f| sheet_of(&f.text()));
    let workbook = chart_external_data(&root);
    Some(PartChart { kind: model_kind(first), data, title, legend, data_labels, sheet, workbook })
}

/// The `c:externalData` relationship id of a chart part's root.
fn chart_external_data(root: &El) -> Option<String> {
    let ext = root.child(Local("externalData"))?;
    ext.attrs.iter().find(|(k, _)| k.ends_with(":id") || k == "id").map(|(_, v)| v.clone())
}

/// The chart type groups (`c:barChart`, `c:lineChart`…) of a plot area.
fn groups(plot: &El) -> impl Iterator<Item = &El> {
    plot.elements().filter(|e| e.local().ends_with("Chart")).take(MAX_GROUPS)
}

/// The series of `groups` in `c:order` order (file order among equal orders).
fn ordered<'a>(groups: &[&'a El]) -> Vec<&'a El> {
    let mut sers: Vec<(u32, usize, &El)> = groups
        .iter()
        .flat_map(|g| g.children(Local("ser")))
        .take(MAX_SERIES)
        .enumerate()
        .map(|(i, s)| (order_of(s).unwrap_or(i as u32), i, s))
        .collect();
    sers.sort_by_key(|(o, i, _)| (*o, *i));
    sers.into_iter().map(|(_, _, s)| s).collect()
}

fn order_of(ser: &El) -> Option<u32> {
    ser.child(Local("order")).and_then(|o| o.attr_u32("val"))
}

fn idx_of(ser: &El) -> Option<u32> {
    ser.child(Local("idx")).and_then(|o| o.attr_u32("val"))
}

/// The model type nearest a chart type group.
fn model_kind(group: &El) -> ChartType {
    let val = |n: &str| group.child(Local(n)).and_then(|e| e.attr("val")).map(str::to_string);
    let stacked = matches!(val("grouping").as_deref(), Some("stacked" | "percentStacked"));
    match group.local() {
        "barChart" | "bar3DChart" => match (val("barDir").as_deref() == Some("bar"), stacked) {
            (true, true) => ChartType::BarStacked,
            (true, false) => ChartType::Bar,
            (false, true) => ChartType::ColumnStacked,
            (false, false) => ChartType::Column,
        },
        "lineChart" | "line3DChart" | "stockChart" | "radarChart" => ChartType::Line,
        "areaChart" | "area3DChart" => ChartType::Area,
        "pieChart" | "pie3DChart" | "ofPieChart" => ChartType::Pie,
        "doughnutChart" => ChartType::Doughnut,
        "scatterChart" | "bubbleChart" => ChartType::Scatter,
        _ => ChartType::Column,
    }
}

/// A series' name: its `c:tx` (cached text or a literal `c:v`).
fn series_name(ser: &El) -> String {
    let Some(tx) = ser.child(Local("tx")) else { return String::new() };
    let text = match tx.child(Local("v")) {
        Some(v) => v.text().into_owned(),
        None => tx.find(Local("v")).map(|v| v.text().into_owned()).unwrap_or_default(),
    };
    clean_text(&text)
}

/// The point texts of a category element (`c:cat`, `c:xVal`): its string or number cache or
/// literal; a multi-level category takes its innermost level.
fn cell_texts(el: &El) -> Vec<String> {
    let Some(src) = el.elements().next() else { return Vec::new() };
    let holder = match src.local() {
        "strRef" => src.child(Local("strCache")),
        "numRef" => src.child(Local("numCache")),
        "multiLvlStrRef" => src.child(Local("multiLvlStrCache")).and_then(|c| c.child(Local("lvl"))),
        "strLit" | "numLit" => Some(src),
        _ => None,
    };
    holder.map(|h| points(h).into_iter().map(|p| p.unwrap_or_default()).collect()).unwrap_or_default()
}

/// The values of a value element (`c:val`, `c:yVal`): its number cache or literal.
fn numbers(el: &El) -> Vec<Option<f64>> {
    let Some(src) = el.elements().next() else { return Vec::new() };
    let holder = match src.local() {
        "numRef" => src.child(Local("numCache")),
        "numLit" => Some(src),
        _ => None,
    };
    holder
        .map(|h| points(h).into_iter().map(|p| p.and_then(|t| t.trim().parse::<f64>().ok()).filter(|v| v.is_finite())).collect())
        .unwrap_or_default()
}

/// The points of a cache or literal (`c:ptCount`, `c:pt idx`/`c:v`), in place order; a place
/// without a point is `None`.
fn points(cache: &El) -> Vec<Option<String>> {
    let count = cache.child(Local("ptCount")).and_then(|c| c.attr_u32("val")).map_or(0, |c| c as usize).min(MAX_POINTS);
    let mut out: Vec<Option<String>> = vec![None; count];
    for pt in cache.children(Local("pt")) {
        let Some(i) = pt.attr_u32("idx").map(|i| i as usize).filter(|i| *i < MAX_POINTS) else { continue };
        if i >= out.len() {
            out.resize(i + 1, None);
        }
        if let Some(slot) = out.get_mut(i) {
            *slot = pt.child(Local("v")).map(|v| v.text().into_owned());
        }
    }
    out
}

/// The title's text: its rich text, its cached reference, or (an automatic title) the only
/// series' name.
fn title_text(chart: &El, data: &ChartData) -> Option<String> {
    let title = chart.child(Local("title"))?;
    if let Some(tx) = title.child(Local("tx")) {
        let text = match tx.child(Local("rich")) {
            Some(rich) => rich
                .children(Local("p"))
                .map(|p| p.find_all(Local("t")).map(|t| t.text().into_owned()).collect::<String>())
                .collect::<Vec<_>>()
                .join(" "),
            None => tx.find(Local("v")).map(|v| v.text().into_owned()).unwrap_or_default(),
        };
        return Some(clean_text(&text));
    }
    let deleted = chart.child(Local("autoTitleDeleted")).and_then(|d| d.attr("val")).is_some_and(truthy);
    match data.series.as_slice() {
        [only] if !deleted => Some(only.name.clone()),
        _ => Some("Chart Title".to_string()),
    }
}

fn truthy(v: &str) -> bool {
    v == "1" || v.eq_ignore_ascii_case("true")
}

/// Whether a `c:dLbls` shows values.
fn shows_values(d: Option<&El>) -> bool {
    d.and_then(|d| d.child(Local("showVal"))).and_then(|v| v.attr("val")).is_some_and(truthy)
}

/// The sheet a cell reference names (`Sheet1!$B$2:$B$5` → `Sheet1`).
fn sheet_of(f: &str) -> Option<String> {
    let (sheet, _) = f.trim().trim_start_matches('(').rsplit_once('!')?;
    let sheet = sheet.trim();
    (!sheet.is_empty() && !sheet.contains(',')).then(|| sheet.to_string())
}

/// A sheet name as a reference writes it: quoted when it isn't a plain name.
pub fn sheet_ref(name: &str) -> String {
    let plain =
        !name.is_empty() && name.chars().all(|c| c.is_alphanumeric() || c == '_' || c == '.') && !name.starts_with(|c: char| c.is_ascii_digit());
    if plain { name.to_string() } else { format!("'{}'", name.replace('\'', "''")) }
}

/// A sheet name from its reference form (`'My ''data'''` → `My 'data'`).
pub fn sheet_name(reference: &str) -> String {
    let r = reference.trim();
    match r.strip_prefix('\'').and_then(|r| r.strip_suffix('\'')) {
        Some(inner) => inner.replace("''", "'"),
        None => r.to_string(),
    }
}

/// The letters of column `i` (0: `A`, 26: `AA`).
pub fn column(i: usize) -> String {
    let mut n = i + 1;
    let mut s = Vec::new();
    while n > 0 {
        let r = (n - 1) % 26;
        s.push(char::from(b'A' + r as u8));
        n = (n - 1) / 26;
    }
    s.iter().rev().collect()
}

/// A number as a chart part or sheet writes it.
pub fn number_text(v: f64) -> String {
    if v == v.trunc() && v.abs() < 1e15 { format!("{}", v as i64) } else { format!("{v}") }
}

/// The chart part `bytes` with `edit` made: a complete XML document.
pub fn edit_part(bytes: &[u8], edit: &PartEdit) -> Result<Vec<u8>, EditError> {
    let Some(mut root) = parse(bytes) else { return err("this isn't a chart WordCraft can read") };
    let c = prefix_of(&root);
    let sheet = root.find(Local("f")).and_then(|f| sheet_of(&f.text()));
    let Some(chart) = root.child_mut(Local("chart")) else { return err("the chart has no chart element") };
    match edit {
        PartEdit::Data { categories, series } => set_data(chart, &c, sheet.as_deref(), categories.as_deref(), series.as_deref())?,
        PartEdit::Title(text) => set_title(chart, &c, text.as_deref()),
        PartEdit::Legend(pos) => set_legend(chart, &c, *pos),
        PartEdit::DataLabels(on) => set_data_labels(chart, &c, *on)?,
    }
    if edit_needs_drawing_ns(edit) && !has_ns(&root, "a") {
        root.set_attr("xmlns:a", "http://schemas.openxmlformats.org/drawingml/2006/main");
    }
    let mut out = craft_xml::DECL_STANDALONE.to_string();
    out.push_str(&root.to_xml());
    Ok(out.into_bytes())
}

fn edit_needs_drawing_ns(edit: &PartEdit) -> bool {
    matches!(edit, PartEdit::Title(Some(_)))
}

fn has_ns(root: &El, prefix: &str) -> bool {
    root.attr(&format!("xmlns:{prefix}")).is_some()
}

/// The prefix the part names chart elements with, with its colon (`c:`; empty for none).
fn prefix_of(root: &El) -> String {
    match root.prefix() {
        "" => String::new(),
        p => format!("{p}:"),
    }
}

/// A chart element named `local` in the part's prefix.
fn el(c: &str, local: &str) -> El {
    El::new(format!("{c}{local}"))
}

/// A chart element with a `val` attribute.
fn val_el(c: &str, local: &str, v: impl ToString) -> El {
    el(c, local).with_attr("val", v)
}

/// The place of the first child matching `local`.
fn pos(e: &El, local: &str) -> Option<usize> {
    e.children.iter().position(|n| matches!(n, Node::El(x) if x.local() == local))
}

/// Remove every child element matching `local`.
fn remove_all(e: &mut El, local: &str) {
    e.children.retain(|n| !matches!(n, Node::El(x) if x.local() == local));
}

// ------------------------------------------------------------------------------------- data

/// Where each series' cells go and how its caches are written.
struct Layout<'a> {
    c: &'a str,
    /// The sheet reference (`Sheet1`), when the part refers to a sheet.
    sheet: Option<&'a str>,
    /// Rows of data.
    n: usize,
}

impl Layout<'_> {
    fn range(&self, col: usize, from: usize, to: usize) -> Option<String> {
        let sheet = self.sheet?;
        let c = column(col);
        Some(if from == to { format!("{sheet}!${c}${from}") } else { format!("{sheet}!${c}${from}:${c}${to}") })
    }
}

fn set_data(chart: &mut El, c: &str, sheet: Option<&str>, cats: Option<&[String]>, series: Option<&[SeriesData]>) -> Result<(), EditError> {
    let Some(plot) = chart.child_mut(Local("plotArea")) else { return err("the chart has no plot area") };
    let current = current_data(plot);
    let data = ChartData {
        categories: cats.map_or_else(|| current.categories.clone(), <[String]>::to_vec),
        series: series.map_or_else(|| current.series.clone(), <[SeriesData]>::to_vec),
    }
    .bounded();
    if data.series.is_empty() {
        return err("a chart needs at least one series");
    }
    fit_series_count(plot, c, data.series.len())?;
    let layout = Layout { c, sheet, n: data.points() };
    let scatter = groups(plot).next().is_some_and(|g| matches!(g.local(), "scatterChart" | "bubbleChart"));
    let mut place = 0usize;
    for g in plot.elements_mut().filter(|e| e.local().ends_with("Chart")).take(MAX_GROUPS) {
        let order = group_order(g);
        for (i, rank) in order {
            let Some(s) = data.series.get(rank) else { continue };
            if let Some(ser) = g.elements_mut().filter(|e| e.local() == "ser").nth(i) {
                write_series(ser, &layout, rank, s, &data.categories, scatter);
                place += 1;
            }
        }
    }
    if place == 0 {
        return err("the chart's series couldn't be matched");
    }
    Ok(())
}

/// The data the plot area holds now.
fn current_data(plot: &El) -> ChartData {
    let gs: Vec<&El> = groups(plot).collect();
    let scatter = gs.first().is_some_and(|g| matches!(g.local(), "scatterChart" | "bubbleChart"));
    let sers = ordered(&gs);
    ChartData {
        categories: sers.iter().find_map(|s| s.child(Local(if scatter { "xVal" } else { "cat" }))).map(cell_texts).unwrap_or_default(),
        series: sers
            .iter()
            .map(|s| SeriesData {
                name: series_name(s),
                values: s.child(Local(if scatter { "yVal" } else { "val" })).map(numbers).unwrap_or_default(),
            })
            .collect(),
    }
}

/// For each series of group `g` (by its place in the group), its rank in the whole chart's
/// `c:order` order. Ranks are found against every group, so the plot area is read again.
fn group_order(g: &El) -> Vec<(usize, usize)> {
    // Ranks are stored on the series by `fit_series_count` as their `c:order`, which is then
    // 0..n across the chart.
    g.children(Local("ser")).enumerate().filter_map(|(i, s)| order_of(s).map(|o| (i, o as usize))).collect()
}

/// Make the chart hold `want` series: drop the last ones in `c:order` order (and chart type
/// groups left empty, with axes only they used), or add copies of the last one. Afterwards the
/// series' `c:order` values are 0, 1, 2… in their old order, and their `c:idx` values are unique.
fn fit_series_count(plot: &mut El, c: &str, want: usize) -> Result<(), EditError> {
    // (group place among plot children, series place among group children, order, file index)
    let mut all: Vec<(usize, usize, u32, usize)> = Vec::new();
    let mut k = 0usize;
    for (gi, g) in plot.children.iter().enumerate() {
        let Node::El(g) = g else { continue };
        if !g.local().ends_with("Chart") {
            continue;
        }
        for (si, s) in g.children.iter().enumerate() {
            if let Node::El(s) = s
                && s.local() == "ser"
            {
                all.push((gi, si, order_of(s).unwrap_or(k as u32), k));
                k += 1;
            }
        }
    }
    if all.is_empty() {
        return err("the chart has no series");
    }
    all.sort_by_key(|(_, _, o, k)| (*o, *k));
    let have = all.len();
    if want < have {
        let mut gone: Vec<(usize, usize)> = all.iter().skip(want).map(|(g, s, _, _)| (*g, *s)).collect();
        gone.sort_unstable_by(|a, b| b.cmp(a));
        for (g, s) in gone {
            if let Some(Node::El(group)) = plot.children.get_mut(g) {
                group.children.remove(s);
            }
        }
        all.truncate(want);
    }
    // Renumber the kept series 0.. in their order.
    for (rank, (g, s, _, _)) in all.iter().enumerate() {
        if let Some(Node::El(group)) = plot.children.get_mut(*g)
            && let Some(Node::El(ser)) = group.children.get_mut(*s)
        {
            set_child_val(ser, c, "order", rank, 1);
        }
    }
    if want > have {
        add_series(plot, c, all.last().map(|(g, s, _, _)| (*g, *s)), have, want)?;
    }
    drop_empty_groups(plot);
    unique_idx(plot, c);
    Ok(())
}

/// Copies of the series at `last` (group place, series place), ranked `from..to`, after it.
fn add_series(plot: &mut El, c: &str, last: Option<(usize, usize)>, from: usize, to: usize) -> Result<(), EditError> {
    let Some((g, s)) = last else { return err("the chart has no series to copy") };
    let Some(Node::El(group)) = plot.children.get_mut(g) else { return err("the chart's series couldn't be found") };
    let Some(Node::El(template)) = group.children.get(s).cloned() else { return err("the chart's series couldn't be found") };
    for (at, rank) in (s + 1..).zip(from..to) {
        let mut ser = template.clone();
        // A new series takes the next accent colour, as Word gives it: no colours of its own.
        for local in ["spPr", "dPt", "dLbls", "trendline", "errBars", "extLst"] {
            remove_all(&mut ser, local);
        }
        if let Some(m) = ser.child_mut(Local("marker")) {
            remove_all(m, "spPr");
        }
        set_child_val(&mut ser, c, "order", rank, 1);
        group.children.insert(at, Node::El(ser));
    }
    Ok(())
}

/// Remove chart type groups without series, and axes no remaining group uses.
fn drop_empty_groups(plot: &mut El) {
    let empty = |e: &El| e.local().ends_with("Chart") && e.child(Local("ser")).is_none();
    let had_groups = plot.elements().any(|e| e.local().ends_with("Chart"));
    plot.children.retain(|n| !matches!(n, Node::El(e) if empty(e)));
    if !had_groups {
        return;
    }
    let used: Vec<String> = plot
        .elements()
        .filter(|e| e.local().ends_with("Chart"))
        .flat_map(|g| g.children(Local("axId")).filter_map(|a| a.attr("val").map(str::to_string)).collect::<Vec<_>>())
        .collect();
    plot.children.retain(|n| match n {
        Node::El(e) if e.local().ends_with("Ax") => e.child(Local("axId")).and_then(|a| a.attr("val")).is_none_or(|v| used.iter().any(|u| u == v)),
        _ => true,
    });
}

/// Give series that share a `c:idx` (a copy) the next free ones.
fn unique_idx(plot: &mut El, c: &str) {
    let mut seen: Vec<u32> = Vec::new();
    let mut next = plot.elements().flat_map(|g| g.children(Local("ser"))).filter_map(idx_of).max().map_or(0, |m| m.saturating_add(1));
    for g in plot.elements_mut().filter(|e| e.local().ends_with("Chart")) {
        for ser in g.elements_mut().filter(|e| e.local() == "ser") {
            match idx_of(ser) {
                Some(i) if !seen.contains(&i) => seen.push(i),
                _ => {
                    set_child_val(ser, c, "idx", next, 0);
                    seen.push(next);
                    next = next.saturating_add(1);
                }
            }
        }
    }
}

/// Set child `local`'s `val`, adding it at `place` when it is missing.
fn set_child_val(e: &mut El, c: &str, local: &str, v: impl ToString, place: usize) {
    match e.child_mut(Local(local)) {
        Some(x) => x.set_attr("val", v),
        None => {
            let at = place.min(e.children.len());
            e.children.insert(at, Node::El(val_el(c, local, v)));
        }
    }
}

/// Write series `rank`'s name, categories and values into `ser`.
fn write_series(ser: &mut El, l: &Layout<'_>, rank: usize, s: &SeriesData, cats: &[String], scatter: bool) {
    let col = rank + 1;
    write_name(ser, l, col, &s.name);
    let (cat_name, val_name) = if scatter { ("xVal", "yVal") } else { ("cat", "val") };
    let cat = categories_el(ser.child(Local(cat_name)), l, cat_name, cats);
    let mut vals: Vec<Option<f64>> = s.values.clone();
    vals.resize(l.n, None);
    let val = values_el(ser.child(Local(val_name)), l, val_name, col, &vals);
    replace_or_insert(ser, cat_name, cat, Some(val_name));
    replace_or_insert(ser, val_name, val, None);
    if let Some(b) = ser.child_mut(Local("bubbleSize")) {
        resize_cache(b, l.n);
    }
    // Points past the data lose their own formatting and labels.
    let keep = |e: &El| e.child(Local("idx")).and_then(|i| i.attr_u32("val")).is_none_or(|i| (i as usize) < l.n);
    ser.children.retain(|n| !matches!(n, Node::El(e) if e.local() == "dPt" && !keep(e)));
    if let Some(d) = ser.child_mut(Local("dLbls")) {
        d.children.retain(|n| !matches!(n, Node::El(e) if e.local() == "dLbl" && !keep(e)));
    }
}

/// Replace child `local` with `new`, or insert it before `before` (or at the end).
fn replace_or_insert(e: &mut El, local: &str, new: El, before: Option<&str>) {
    if let Some(i) = pos(e, local) {
        if let Some(slot) = e.children.get_mut(i) {
            *slot = Node::El(new);
        }
        return;
    }
    let at = before.and_then(|b| pos(e, b)).or_else(|| pos(e, "smooth")).or_else(|| pos(e, "extLst")).unwrap_or(e.children.len());
    e.children.insert(at, Node::El(new));
}

/// The series name in `c:tx`: a reference to its header cell when the chart has a sheet, or a
/// literal.
fn write_name(ser: &mut El, l: &Layout<'_>, col: usize, name: &str) {
    let c = l.c;
    let tx = match l.range(col, 1, 1) {
        Some(f) => el(c, "tx").with_child(el(c, "strRef").with_child(el(c, "f").with_text(f)).with_child(str_cache(c, &[name.to_string()]))),
        None => el(c, "tx").with_child(el(c, "v").with_text(name)),
    };
    if pos(ser, "tx").is_some() {
        replace_or_insert(ser, "tx", tx, None);
    } else {
        let at = pos(ser, "order").map_or(0, |i| i + 1);
        ser.children.insert(at.min(ser.children.len()), Node::El(tx));
    }
}

fn str_cache(c: &str, texts: &[String]) -> El {
    let mut cache = el(c, "strCache").with_child(val_el(c, "ptCount", texts.len()));
    for (i, t) in texts.iter().enumerate() {
        cache.push(el(c, "pt").with_attr("idx", i).with_child(el(c, "v").with_text(t.as_str())));
    }
    cache
}

fn num_points(c: &str, mut holder: El, values: &[Option<f64>]) -> El {
    holder.push(val_el(c, "ptCount", values.len()));
    for (i, v) in values.iter().enumerate() {
        if let Some(v) = v {
            holder.push(el(c, "pt").with_attr("idx", i).with_child(el(c, "v").with_text(number_text(*v))));
        }
    }
    holder
}

/// The format code of an old number cache or literal (`General` without one).
fn format_code(old: Option<&El>) -> String {
    old.and_then(|o| o.find(Local("formatCode"))).map(|f| f.text().into_owned()).filter(|f| !f.is_empty()).unwrap_or_else(|| "General".to_string())
}

/// The categories element: numbers stay numbers (a date axis keeps its dates) when every new
/// category reads as one, otherwise text.
fn categories_el(old: Option<&El>, l: &Layout<'_>, local: &str, cats: &[String]) -> El {
    let c = l.c;
    let mut texts: Vec<String> = cats.to_vec();
    texts.resize(l.n, String::new());
    let was_number = old.and_then(|o| o.elements().next()).is_some_and(|s| matches!(s.local(), "numRef" | "numLit"));
    let nums: Option<Vec<Option<f64>>> = texts
        .iter()
        .map(|t| if t.trim().is_empty() { Some(None) } else { t.trim().parse::<f64>().ok().filter(|v| v.is_finite()).map(Some) })
        .collect();
    let range = l.range(0, 2, l.n + 1);
    let body = match (was_number, nums, range) {
        (true, Some(nums), Some(f)) => {
            let cache = num_points(c, el(c, "numCache").with_child(el(c, "formatCode").with_text(format_code(old))), &nums);
            el(c, "numRef").with_child(el(c, "f").with_text(f)).with_child(cache)
        }
        (true, Some(nums), None) => num_points(c, el(c, "numLit").with_child(el(c, "formatCode").with_text(format_code(old))), &nums),
        (_, _, Some(f)) => el(c, "strRef").with_child(el(c, "f").with_text(f)).with_child(str_cache(c, &texts)),
        (_, _, None) => {
            let mut lit = str_cache(c, &texts);
            lit.name = format!("{c}strLit");
            lit
        }
    };
    el(c, local).with_child(body)
}

/// The values element of series column `col`.
fn values_el(old: Option<&El>, l: &Layout<'_>, local: &str, col: usize, values: &[Option<f64>]) -> El {
    let c = l.c;
    let fmt = format_code(old);
    let body = match l.range(col, 2, l.n + 1) {
        Some(f) => {
            let cache = num_points(c, el(c, "numCache").with_child(el(c, "formatCode").with_text(fmt)), values);
            el(c, "numRef").with_child(el(c, "f").with_text(f)).with_child(cache)
        }
        None => num_points(c, el(c, "numLit").with_child(el(c, "formatCode").with_text(fmt)), values),
    };
    el(c, local).with_child(body)
}

/// Cut or pad a number cache (bubble sizes) to `n` points.
fn resize_cache(e: &mut El, n: usize) {
    let Some(holder) = e.elements_mut().next() else { return };
    let holder = if holder.local() == "numRef" {
        match holder.child_mut(Local("numCache")) {
            Some(h) => h,
            None => return,
        }
    } else {
        holder
    };
    if let Some(count) = holder.child_mut(Local("ptCount")) {
        count.set_attr("val", n);
    }
    holder.children.retain(|node| match node {
        Node::El(p) if p.local() == "pt" => p.attr_u32("idx").is_some_and(|i| (i as usize) < n),
        _ => true,
    });
}

// ------------------------------------------------------------------------- title and legend

fn set_title(chart: &mut El, c: &str, text: Option<&str>) {
    let Some(text) = text else {
        remove_all(chart, "title");
        set_child_val(chart, c, "autoTitleDeleted", 1, 0);
        return;
    };
    let text = clean_text(text);
    let old_rich = chart.child(Local("title")).and_then(|t| t.child(Local("tx"))).and_then(|tx| tx.child(Local("rich"))).cloned();
    let tx = el(c, "tx").with_child(rich(c, old_rich.as_ref(), &text));
    match chart.child_mut(Local("title")) {
        Some(title) => {
            remove_all(title, "tx");
            title.children.insert(0, Node::El(tx));
        }
        None => {
            let title = el(c, "title").with_child(tx).with_child(val_el(c, "overlay", 0));
            chart.children.insert(0, Node::El(title));
        }
    }
    set_child_val(chart, c, "autoTitleDeleted", 0, 1);
}

/// A `c:rich` holding `text`, keeping the body, list style, paragraph and run properties of
/// `old` (so the title keeps its font, size and colour).
fn rich(c: &str, old: Option<&El>, text: &str) -> El {
    let keep = |local: &str| old.and_then(|o| o.child(Local(local))).cloned();
    let first_p = old.and_then(|o| o.child(Local("p")));
    let mut r = El::new("a:r");
    if let Some(rpr) = first_p.and_then(|p| p.find(Local("rPr"))).or_else(|| first_p.and_then(|p| p.find(Local("defRPr")))) {
        let mut rpr = rpr.clone();
        rpr.name = "a:rPr".to_string();
        rpr.children.retain(|n| matches!(n, Node::El(_)));
        r.push(rpr);
    }
    r.push(El::new("a:t").with_text(text));
    let mut p = El::new("a:p");
    if let Some(ppr) = first_p.and_then(|p| p.child(Local("pPr"))) {
        p.push(ppr.clone());
    }
    p.push(r);
    el(c, "rich")
        .with_child(keep("bodyPr").unwrap_or_else(|| El::new("a:bodyPr")))
        .with_child(keep("lstStyle").unwrap_or_else(|| El::new("a:lstStyle")))
        .with_child(p)
}

fn set_legend(chart: &mut El, c: &str, pos_: Option<LegendPos>) {
    let Some(p) = pos_ else {
        remove_all(chart, "legend");
        return;
    };
    let code = match p {
        LegendPos::Right => "r",
        LegendPos::Left => "l",
        LegendPos::Top => "t",
        LegendPos::Bottom => "b",
    };
    match chart.child_mut(Local("legend")) {
        Some(legend) => {
            set_child_val(legend, c, "legendPos", code, 0);
            // A manual layout would keep the legend where it was.
            remove_all(legend, "layout");
        }
        None => {
            let legend = el(c, "legend").with_child(val_el(c, "legendPos", code)).with_child(val_el(c, "overlay", 0));
            let at = pos(chart, "plotArea").map_or(chart.children.len(), |i| i + 1);
            chart.children.insert(at, Node::El(legend));
        }
    }
}

fn set_data_labels(chart: &mut El, c: &str, on: bool) -> Result<(), EditError> {
    let Some(plot) = chart.child_mut(Local("plotArea")) else { return err("the chart has no plot area") };
    for g in plot.elements_mut().filter(|e| e.local().ends_with("Chart")).take(MAX_GROUPS) {
        // The chart's setting replaces each series' own, as for WordCraft's charts.
        for ser in g.elements_mut().filter(|e| e.local() == "ser") {
            remove_all(ser, "dLbls");
        }
        let labels = data_labels_el(c, g.child(Local("dLbls")), on);
        match pos(g, "dLbls") {
            Some(i) => {
                if let Some(slot) = g.children.get_mut(i) {
                    *slot = Node::El(labels);
                }
            }
            None => {
                let at = g.children.iter().rposition(|n| matches!(n, Node::El(e) if e.local() == "ser")).map_or(g.children.len(), |i| i + 1);
                g.children.insert(at, Node::El(labels));
            }
        }
    }
    Ok(())
}

/// A group's `c:dLbls` showing values or not, keeping the number format, look and position of
/// `old`.
fn data_labels_el(c: &str, old: Option<&El>, on: bool) -> El {
    let mut d = el(c, "dLbls");
    if let Some(old) = old {
        for local in ["numFmt", "spPr", "txPr", "dLblPos"] {
            if let Some(x) = old.child(Local(local)) {
                d.push(x.clone());
            }
        }
    }
    let shown = |local: &str| old.and_then(|o| o.child(Local(local))).and_then(|x| x.attr("val")).is_some_and(truthy);
    d.push(val_el(c, "showLegendKey", u8::from(shown("showLegendKey"))));
    d.push(val_el(c, "showVal", u8::from(on)));
    for local in ["showCatName", "showSerName", "showPercent", "showBubbleSize"] {
        d.push(val_el(c, local, u8::from(on && shown(local))));
    }
    if let Some(old) = old {
        for local in ["separator", "showLeaderLines", "leaderLines", "extLst"] {
            if let Some(x) = old.child(Local(local)) {
                d.push(x.clone());
            }
        }
    }
    d
}
