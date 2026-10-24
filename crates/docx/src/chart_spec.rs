//! Charts WordCraft makes ([`ChartSpec`]) as chart parts (ECMA-376 Part 1 §21.2) and back.
//!
//! [`chart_xml`] writes a `c:chartSpace` with the data as literals in the part itself
//! (`c:strLit`, `c:numLit`): no embedded workbook (`c:externalData`) and no external references.
//! [`chart_items`] draws a chart through the same code that draws chart parts read from files.
//! [`spec_of`] turns a chart part back into the model only when writing that model again gives
//! the same markup, so a chart with anything the model leaves out stays as it was read.

use wordcraft_doc::chart::{ChartAxis, ChartSeries, ChartSpec, ChartType, LegendPos};
use wordcraft_doc::graphic::GraphicItem;
use wordcraft_doc::props::Rgb;

use crate::xml::{self, El, MAX_DEPTH, Node, W};

const NS_C: &str = "http://schemas.openxmlformats.org/drawingml/2006/chart";
const NS_A: &str = "http://schemas.openxmlformats.org/drawingml/2006/main";
const NS_R: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships";
/// Content type of a chart part.
pub(crate) const CHART_CT: &str = "application/vnd.openxmlformats-officedocument.drawingml.chart+xml";
/// `a:graphicData` URI of a chart.
pub(crate) const CHART_URI: &str = "http://schemas.openxmlformats.org/drawingml/2006/chart";
/// Axis ids (any pair of distinct numbers will do; Word keeps them).
const CAT_AX: &str = "500000001";
const VAL_AX: &str = "500000002";

/// The chart part for `spec`: a complete XML document. `spec` should be sanitized
/// ([`ChartSpec::sanitize`]); a value that isn't finite is written as a gap either way.
pub fn chart_xml(spec: &ChartSpec) -> String {
    let mut w = W::new();
    w.open("c:chartSpace", &[("xmlns:c", NS_C), ("xmlns:a", NS_A), ("xmlns:r", NS_R)]);
    w.empty("c:roundedCorners", &[("val", "0")]);
    w.open("c:chart", &[]);
    if let Some(t) = &spec.title {
        w.open("c:title", &[]);
        w.open("c:tx", &[]);
        w.open("c:rich", &[]);
        w.empty("a:bodyPr", &[]);
        w.empty("a:lstStyle", &[]);
        w.open("a:p", &[]);
        w.open("a:r", &[]);
        w.leaf("a:t", &[], t);
        w.close("a:r");
        w.close("a:p");
        w.close("c:rich");
        w.close("c:tx");
        w.empty("c:overlay", &[("val", "0")]);
        w.close("c:title");
    }
    w.empty("c:autoTitleDeleted", &[("val", if spec.title.is_some() { "0" } else { "1" })]);
    w.open("c:plotArea", &[]);
    w.empty("c:layout", &[]);
    plot(&mut w, spec);
    w.close("c:plotArea");
    if let Some(pos) = spec.legend {
        w.open("c:legend", &[]);
        let p = match pos {
            LegendPos::Right => "r",
            LegendPos::Left => "l",
            LegendPos::Top => "t",
            LegendPos::Bottom => "b",
        };
        w.empty("c:legendPos", &[("val", p)]);
        w.empty("c:overlay", &[("val", "0")]);
        w.close("c:legend");
    }
    w.empty("c:plotVisOnly", &[("val", "1")]);
    w.empty("c:dispBlanksAs", &[("val", "gap")]);
    w.close("c:chart");
    // A white chart area without a border, in the document's own background colour.
    w.open("c:spPr", &[]);
    w.open("a:solidFill", &[]);
    w.empty("a:schemeClr", &[("val", "bg1")]);
    w.close("a:solidFill");
    w.open("a:ln", &[]);
    w.empty("a:noFill", &[]);
    w.close("a:ln");
    w.close("c:spPr");
    w.close("c:chartSpace");
    w.s
}

/// What `spec` draws as, `w` × `h` points, in the colours of `theme` (series take the accents in
/// turn). Empty when the chart has no series.
pub fn chart_items(spec: &ChartSpec, theme: &[Rgb], w: f32, h: f32) -> Vec<GraphicItem> {
    match xml::parse(chart_xml(spec).as_bytes()) {
        Ok(space) => crate::read::chart::chart_items(&space, theme, w, h),
        Err(_) => Vec::new(),
    }
}

/// The chart type element and the series in it, then the axes.
fn plot(w: &mut W, spec: &ChartSpec) {
    let k = spec.kind;
    let el = match k {
        ChartType::Column | ChartType::ColumnStacked | ChartType::Bar | ChartType::BarStacked => "c:barChart",
        ChartType::Line => "c:lineChart",
        ChartType::Area => "c:areaChart",
        ChartType::Pie => "c:pieChart",
        ChartType::Doughnut => "c:doughnutChart",
        ChartType::Scatter => "c:scatterChart",
    };
    let stacked = matches!(k, ChartType::ColumnStacked | ChartType::BarStacked);
    let horizontal = matches!(k, ChartType::Bar | ChartType::BarStacked);
    w.open(el, &[]);
    match k {
        ChartType::Column | ChartType::ColumnStacked | ChartType::Bar | ChartType::BarStacked => {
            w.empty("c:barDir", &[("val", if horizontal { "bar" } else { "col" })]);
            w.empty("c:grouping", &[("val", if stacked { "stacked" } else { "clustered" })]);
        }
        ChartType::Line | ChartType::Area => w.empty("c:grouping", &[("val", "standard")]),
        ChartType::Scatter => w.empty("c:scatterStyle", &[("val", "lineMarker")]),
        ChartType::Pie | ChartType::Doughnut => {}
    }
    w.empty("c:varyColors", &[("val", if k.is_round() { "1" } else { "0" })]);
    // Pie and doughnut charts show the first series; the others are kept for other types.
    for (i, s) in spec.series.iter().enumerate() {
        series(w, spec, i, s);
    }
    if spec.data_labels {
        w.open("c:dLbls", &[]);
        for (name, on) in [
            ("c:showLegendKey", false),
            ("c:showVal", true),
            ("c:showCatName", false),
            ("c:showSerName", false),
            ("c:showPercent", false),
            ("c:showBubbleSize", false),
        ] {
            w.empty(name, &[("val", if on { "1" } else { "0" })]);
        }
        w.close("c:dLbls");
    }
    match k {
        ChartType::Column | ChartType::ColumnStacked | ChartType::Bar | ChartType::BarStacked => {
            w.empty("c:gapWidth", &[("val", "150")]);
            if stacked {
                w.empty("c:overlap", &[("val", "100")]);
            }
        }
        ChartType::Line => w.empty("c:marker", &[("val", "1")]),
        ChartType::Pie => w.empty("c:firstSliceAng", &[("val", "0")]),
        ChartType::Doughnut => {
            w.empty("c:firstSliceAng", &[("val", "0")]);
            w.empty("c:holeSize", &[("val", "50")]);
        }
        ChartType::Area | ChartType::Scatter => {}
    }
    if !k.is_round() {
        w.empty("c:axId", &[("val", CAT_AX)]);
        w.empty("c:axId", &[("val", VAL_AX)]);
    }
    w.close(el);
    match k {
        ChartType::Pie | ChartType::Doughnut => {}
        ChartType::Scatter => {
            value_axis(w, CAT_AX, VAL_AX, "b", "midCat", &spec.cat_axis);
            value_axis(w, VAL_AX, CAT_AX, "l", "midCat", &spec.val_axis);
        }
        _ => {
            category_axis(w, if horizontal { "l" } else { "b" }, &spec.cat_axis);
            value_axis(w, VAL_AX, CAT_AX, if horizontal { "b" } else { "l" }, "between", &spec.val_axis);
        }
    }
}

/// One `c:ser` (ECMA-376 element order: idx, order, tx, spPr, marker, invertIfNegative, dLbls,
/// cat/xVal, val/yVal, smooth): the series' formatting (fill, line, markers, its own labels) sits
/// in `c:spPr`, `c:marker` and `c:dLbls`; the values carry their number format.
fn series(w: &mut W, spec: &ChartSpec, i: usize, s: &ChartSeries) {
    let k = spec.kind;
    let i = i.to_string();
    w.open("c:ser", &[]);
    w.empty("c:idx", &[("val", &i)]);
    w.empty("c:order", &[("val", &i)]);
    if !s.name.is_empty() {
        w.open("c:tx", &[]);
        w.leaf("c:v", &[], &s.name);
        w.close("c:tx");
    }
    series_props(w, s);
    series_marker(w, k, s);
    if let Some(on) = s.labels {
        w.open("c:dLbls", &[]);
        for (name, show) in [
            ("c:showLegendKey", false),
            ("c:showVal", on),
            ("c:showCatName", false),
            ("c:showSerName", false),
            ("c:showPercent", false),
            ("c:showBubbleSize", false),
        ] {
            w.empty(name, &[("val", if show { "1" } else { "0" })]);
        }
        w.close("c:dLbls");
    }
    if matches!(k, ChartType::Column | ChartType::ColumnStacked | ChartType::Bar | ChartType::BarStacked) {
        w.empty("c:invertIfNegative", &[("val", "0")]);
    }
    let code = s.fmt.as_deref().unwrap_or("General");
    if k == ChartType::Scatter {
        let xs: Option<Vec<Option<f64>>> = spec
            .categories
            .iter()
            .map(|c| if c.is_empty() { Some(None) } else { c.parse::<f64>().ok().filter(|v| v.is_finite()).map(Some) })
            .collect();
        w.open("c:xVal", &[]);
        match xs {
            Some(xs) => num_lit(w, &xs, "General"),
            None => str_lit(w, &spec.categories),
        }
        w.close("c:xVal");
        w.open("c:yVal", &[]);
        num_lit(w, &s.values, code);
        w.close("c:yVal");
    } else {
        w.open("c:cat", &[]);
        str_lit(w, &spec.categories);
        w.close("c:cat");
        w.open("c:val", &[]);
        num_lit(w, &s.values, code);
        w.close("c:val");
    }
    if matches!(k, ChartType::Line | ChartType::Scatter) {
        w.empty("c:smooth", &[("val", if s.smooth { "1" } else { "0" })]);
    }
    w.close("c:ser");
}

/// `c:spPr`: the series' fill (`a:solidFill` / `a:noFill`) and line (`a:ln`), written when the
/// model sets them. `a:ln` without `w` is the default width, `w` a fixed one, and `a:noFill`
/// inside it "no line" (a scatter series keeps its markers with no line).
fn series_props(w: &mut W, s: &ChartSeries) {
    let fill_set = s.no_fill || s.fill.is_some();
    let ln_set = s.no_line || s.line_w.is_some();
    if !(fill_set || ln_set) {
        return;
    }
    w.open("c:spPr", &[]);
    if fill_set {
        match &s.fill {
            Some(c) => {
                w.open("a:solidFill", &[]);
                w.empty("a:srgbClr", &[("val", &c.hex())]);
                w.close("a:solidFill");
            }
            None => w.empty("a:noFill", &[]),
        }
    }
    if ln_set {
        let w_attr = s.line_w.map(emu).unwrap_or_default();
        let attrs = [("w", w_attr.as_str())];
        w.open("a:ln", if w_attr.is_empty() { &[] } else { &attrs });
        if s.no_line {
            w.empty("a:noFill", &[]);
        }
        w.close("a:ln");
    }
    w.close("c:spPr");
}

/// `c:marker`: line and scatter series say which symbol each point gets; other types have none.
fn series_marker(w: &mut W, k: ChartType, s: &ChartSeries) {
    match k {
        ChartType::Line => {
            w.open("c:marker", &[]);
            w.empty("c:symbol", &[("val", if s.markers == Some(true) { "circle" } else { "none" })]);
            w.close("c:marker");
        }
        ChartType::Scatter => {
            w.open("c:marker", &[]);
            w.empty("c:symbol", &[("val", if s.markers == Some(false) { "none" } else { "circle" })]);
            w.close("c:marker");
        }
        _ => {}
    }
}

/// A line width in points as the `w` attribute DrawingML wants (EMU).
fn emu(pt: f32) -> String {
    ((pt * 12700.0).round().max(0.0) as i64).to_string()
}

fn str_lit(w: &mut W, items: &[String]) {
    w.open("c:strLit", &[]);
    w.empty("c:ptCount", &[("val", &items.len().to_string())]);
    for (i, s) in items.iter().enumerate().filter(|(_, s)| !s.is_empty()) {
        w.open("c:pt", &[("idx", &i.to_string())]);
        w.leaf("c:v", &[], s);
        w.close("c:pt");
    }
    w.close("c:strLit");
}

fn num_lit(w: &mut W, values: &[Option<f64>], code: &str) {
    w.open("c:numLit", &[]);
    w.leaf("c:formatCode", &[], code);
    w.empty("c:ptCount", &[("val", &values.len().to_string())]);
    for (i, v) in values.iter().enumerate() {
        if let Some(v) = v.filter(|v| v.is_finite()) {
            w.open("c:pt", &[("idx", &i.to_string())]);
            w.leaf("c:v", &[], &num(v));
            w.close("c:pt");
        }
    }
    w.close("c:numLit");
}

/// A value as the shortest text that reads back as the same number.
fn num(v: f64) -> String {
    if v == 0.0 { "0".to_string() } else { format!("{v}") }
}

fn category_axis(w: &mut W, pos: &str, ax: &ChartAxis) {
    w.open("c:catAx", &[]);
    w.empty("c:axId", &[("val", CAT_AX)]);
    scaling(w, ax);
    w.empty("c:delete", &[("val", "0")]);
    w.empty("c:axPos", &[("val", pos)]);
    if ax.gridlines {
        w.empty("c:majorGridlines", &[]);
    }
    axis_title(w, ax);
    num_fmt(w, ax);
    w.empty("c:majorTickMark", &[("val", "none")]);
    w.empty("c:minorTickMark", &[("val", "none")]);
    w.empty("c:tickLblPos", &[("val", if ax.labels { "nextTo" } else { "none" })]);
    axis_line(w, ax);
    w.empty("c:crossAx", &[("val", VAL_AX)]);
    w.empty("c:crosses", &[("val", "autoZero")]);
    w.empty("c:auto", &[("val", "1")]);
    w.empty("c:lblAlgn", &[("val", "ctr")]);
    w.empty("c:lblOffset", &[("val", "100")]);
    w.empty("c:noMultiLvlLbl", &[("val", "0")]);
    w.close("c:catAx");
}

/// A value axis (scatter x axes included): its scale, gridlines, title, labels and line, as the
/// model sets them.
fn value_axis(w: &mut W, id: &str, cross: &str, pos: &str, between: &str, ax: &ChartAxis) {
    w.open("c:valAx", &[]);
    w.empty("c:axId", &[("val", id)]);
    scaling(w, ax);
    w.empty("c:delete", &[("val", "0")]);
    w.empty("c:axPos", &[("val", pos)]);
    if ax.gridlines {
        w.empty("c:majorGridlines", &[]);
    }
    axis_title(w, ax);
    num_fmt(w, ax);
    w.empty("c:majorTickMark", &[("val", "none")]);
    w.empty("c:minorTickMark", &[("val", "none")]);
    w.empty("c:tickLblPos", &[("val", if ax.labels { "nextTo" } else { "none" })]);
    axis_line(w, ax);
    w.empty("c:crossAx", &[("val", cross)]);
    w.empty("c:crosses", &[("val", "autoZero")]);
    w.empty("c:crossBetween", &[("val", between)]);
    w.close("c:valAx");
}

/// `c:scaling`: the orientation, then the fixed bounds when the model sets them.
fn scaling(w: &mut W, ax: &ChartAxis) {
    w.open("c:scaling", &[]);
    w.empty("c:orientation", &[("val", "minMax")]);
    for (name, v) in [("c:max", ax.max), ("c:min", ax.min)] {
        if let Some(v) = v.filter(|v| v.is_finite()) {
            w.empty(name, &[("val", &num(v))]);
        }
    }
    w.close("c:scaling");
}

/// The axis title, in the same rich-text shape the chart title uses.
fn axis_title(w: &mut W, ax: &ChartAxis) {
    let Some(t) = &ax.title else { return };
    w.open("c:title", &[]);
    w.open("c:tx", &[]);
    w.open("c:rich", &[]);
    w.empty("a:bodyPr", &[]);
    w.empty("a:lstStyle", &[]);
    w.open("a:p", &[]);
    w.open("a:r", &[]);
    w.leaf("a:t", &[], t);
    w.close("a:r");
    w.close("a:p");
    w.close("c:rich");
    w.close("c:tx");
    w.empty("c:overlay", &[("val", "0")]);
    w.close("c:title");
}

/// Tick-label number format: the model's code, unlinked from the source, else the default.
fn num_fmt(w: &mut W, ax: &ChartAxis) {
    match &ax.fmt {
        Some(code) => w.empty("c:numFmt", &[("formatCode", code), ("sourceLinked", "0")]),
        None => w.empty("c:numFmt", &[("formatCode", "General"), ("sourceLinked", "1")]),
    }
}

/// `c:spPr`: an axis line only when the model keeps it (the default writes nothing, which reads
/// as a line); `a:noFill` when the axis line is off.
fn axis_line(w: &mut W, ax: &ChartAxis) {
    if ax.line {
        return;
    }
    w.open("c:spPr", &[]);
    w.open("a:ln", &[]);
    w.empty("a:noFill", &[]);
    w.close("a:ln");
    w.close("c:spPr");
}

/// The chart a chart part holds, when the part is exactly what [`chart_xml`] writes for it (so
/// editing and saving it loses nothing). `None` for any other chart.
pub(crate) fn spec_of(space: &El) -> Option<ChartSpec> {
    let chart = space.child("c:chart")?;
    let plot = chart.child("c:plotArea")?;
    let ty = plot.els().find(|e| e.name.ends_with("Chart"))?;
    let val = |e: &El, n: &str| e.child(n).and_then(|c| c.attr("val")).map(str::to_string);
    let bar = |stacked_kind, clustered_kind| if val(ty, "c:grouping").as_deref() == Some("stacked") { stacked_kind } else { clustered_kind };
    let kind = match ty.name.as_str() {
        "c:barChart" if val(ty, "c:barDir").as_deref() == Some("bar") => bar(ChartType::BarStacked, ChartType::Bar),
        "c:barChart" => bar(ChartType::ColumnStacked, ChartType::Column),
        "c:lineChart" => ChartType::Line,
        "c:areaChart" => ChartType::Area,
        "c:pieChart" => ChartType::Pie,
        "c:doughnutChart" => ChartType::Doughnut,
        "c:scatterChart" => ChartType::Scatter,
        _ => return None,
    };
    let scatter = kind == ChartType::Scatter;
    let sers: Vec<&El> = ty.children("c:ser").take(wordcraft_doc::chart::MAX_SERIES + 1).collect();
    // More than the model holds can't be the same chart: don't build it to find out.
    let count = |s: &&El| s.find("c:ptCount").and_then(|c| c.attr("val")).and_then(|v| v.parse::<usize>().ok()).unwrap_or(0);
    let longest = sers.iter().map(count).max().unwrap_or(0);
    if sers.len() > wordcraft_doc::chart::MAX_SERIES
        || longest > wordcraft_doc::chart::MAX_POINTS
        || sers.len().saturating_mul(longest) > wordcraft_doc::chart::MAX_CELLS
    {
        return None;
    }
    let categories = sers.first().and_then(|s| s.child(if scatter { "c:xVal" } else { "c:cat" })).map(literal).unwrap_or_default();
    let series = sers
        .iter()
        .map(|s| {
            let ln = s.child("c:spPr").and_then(|p| p.child("a:ln"));
            let symbol = s.child("c:marker").and_then(|m| m.child("c:symbol")).and_then(|m| m.attr("val"));
            ChartSeries {
                name: s.child("c:tx").and_then(|t| t.child("c:v")).map(|v| v.text()).unwrap_or_default(),
                values: s
                    .child(if scatter { "c:yVal" } else { "c:val" })
                    .map(literal)
                    .unwrap_or_default()
                    .iter()
                    .map(|v| v.parse::<f64>().ok().filter(|v| v.is_finite()))
                    .collect(),
                fill: s
                    .child("c:spPr")
                    .and_then(|p| p.child("a:solidFill"))
                    .and_then(|f| f.child("a:srgbClr"))
                    .and_then(|c| c.attr("val"))
                    .and_then(Rgb::parse),
                no_fill: s.child("c:spPr").and_then(|p| p.child("a:noFill")).is_some(),
                line_w: ln.and_then(|l| l.attr("w")).and_then(|w| w.parse::<f64>().ok()).filter(|w| w.is_finite()).map(|w| (w / 12700.0) as f32),
                no_line: ln.and_then(|l| l.child("a:noFill")).is_some(),
                // Scatter series show markers by default, line series by default show none:
                // only a setting that differs from that default belongs in the model.
                markers: symbol.map(|v| v != "none").filter(|m| *m != scatter),
                smooth: s.child("c:smooth").and_then(|v| v.attr("val")).is_some_and(|v| v == "1"),
                labels: s.child("c:dLbls").and_then(|d| d.child("c:showVal")).and_then(|v| v.attr("val")).map(|v| v == "1"),
                fmt: s
                    .child(if scatter { "c:yVal" } else { "c:val" })
                    .and_then(|v| v.els().find(|e| e.name == "c:numLit"))
                    .and_then(|l| l.child("c:formatCode"))
                    .map(|f| f.text())
                    .filter(|f| *f != "General"),
            }
        })
        .collect();
    let title = chart.child("c:title").and_then(|t| t.find("a:t")).map(|t| t.text());
    let legend = chart.child("c:legend").map(|l| match val(l, "c:legendPos").as_deref() {
        Some("l") => LegendPos::Left,
        Some("t") => LegendPos::Top,
        Some("b") => LegendPos::Bottom,
        _ => LegendPos::Right,
    });
    let data_labels = ty.child("c:dLbls").and_then(|d| d.child("c:showVal")).and_then(|v| v.attr("val")) == Some("1");
    let (cat_a, val_a) = crate::read::chart::chart_axes(plot, scatter);
    let axis = |a: crate::read::chart::Axis| ChartAxis {
        title: a.title,
        min: a.min,
        max: a.max,
        fmt: a.fmt,
        gridlines: a.grid,
        labels: a.labels,
        line: a.line,
    };
    let spec = ChartSpec { kind, categories, series, title, legend, data_labels, cat_axis: axis(cat_a), val_axis: axis(val_a) }.sanitized();
    // Editable only when nothing would be lost: writing the model again gives the same part.
    let again = xml::parse(chart_xml(&spec).as_bytes()).ok()?;
    same(space, &again, 0).then_some(spec)
}

/// The points of a `c:strLit` / `c:numLit` (or a cache) as texts by index, `ptCount` long.
fn literal(e: &El) -> Vec<String> {
    let Some(lit) = e.els().find(|c| matches!(c.name.as_str(), "c:strLit" | "c:numLit")) else { return Vec::new() };
    let n = lit
        .child("c:ptCount")
        .and_then(|c| c.attr("val"))
        .and_then(|v| v.parse::<usize>().ok())
        .unwrap_or(0)
        .min(wordcraft_doc::chart::MAX_POINTS + 1);
    let mut out = vec![String::new(); n];
    for p in lit.children("c:pt") {
        if let Some(slot) = p.attr("idx").and_then(|i| i.parse::<usize>().ok()).and_then(|i| out.get_mut(i)) {
            *slot = p.child("c:v").map(|v| v.text()).unwrap_or_default();
        }
    }
    out
}

/// Whether two elements are the same markup: names, attributes (in any order), child elements in
/// order and text (whitespace-only text between elements ignored).
fn same(a: &El, b: &El, depth: usize) -> bool {
    if depth > MAX_DEPTH || a.name != b.name || a.attrs.len() != b.attrs.len() {
        return false;
    }
    if !a.attrs.iter().all(|(k, v)| b.attr(k) == Some(v.as_str()) && b.attrs.iter().any(|(bk, _)| bk == k)) {
        return false;
    }
    let text = |e: &El| -> String {
        e.kids
            .iter()
            .filter_map(|n| match n {
                Node::Text(t) if !t.trim().is_empty() => Some(t.as_str()),
                _ => None,
            })
            .collect()
    };
    if text(a) != text(b) {
        return false;
    }
    let (mut x, mut y) = (a.els(), b.els());
    loop {
        match (x.next(), y.next()) {
            (None, None) => return true,
            (Some(p), Some(q)) if same(p, q, depth + 1) => {}
            _ => return false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wordcraft_doc::THEME_COLORS;

    fn parse(spec: &ChartSpec) -> El {
        xml::parse(chart_xml(spec).as_bytes()).unwrap()
    }

    /// Children of `e` by name, in order.
    fn names(e: &El) -> Vec<&str> {
        e.els().map(|c| c.name.as_str()).collect()
    }

    #[test]
    fn every_type_reads_back_as_itself() {
        for kind in ChartType::ALL {
            let mut spec = ChartSpec::sample(kind);
            spec.title = Some("Sales & <costs>".into());
            spec.data_labels = true;
            spec.legend = Some(LegendPos::Right);
            spec.series[0].values[1] = None;
            spec.categories[2] = String::new();
            let spec = spec.sanitized();
            assert_eq!(spec_of(&parse(&spec)).as_ref(), Some(&spec), "{kind:?}");
            assert!(!chart_items(&spec, &THEME_COLORS, 300.0, 200.0).is_empty(), "{kind:?} draws");
        }
        let plain = ChartSpec { legend: None, ..ChartSpec::sample(ChartType::Line) };
        assert_eq!(spec_of(&parse(&plain)), Some(plain));
    }

    #[test]
    fn a_chart_with_more_than_the_model_stays_read_only() {
        let spec = ChartSpec::sample(ChartType::Column);
        let x = chart_xml(&spec).replace("<c:roundedCorners val=\"0\"/>", "<c:roundedCorners val=\"0\"/><c:style val=\"2\"/>");
        assert_eq!(spec_of(&xml::parse(x.as_bytes()).unwrap()), None);
        let x = chart_xml(&spec).replace("<c:v>3.2</c:v>", "<c:v>3.20</c:v>");
        assert_eq!(spec_of(&xml::parse(x.as_bytes()).unwrap()), None);
    }

    #[test]
    fn elements_come_in_schema_order() {
        let mut spec = ChartSpec::sample(ChartType::ColumnStacked);
        spec.title = Some("T".into());
        spec.data_labels = true;
        let space = parse(&spec);
        assert_eq!(names(&space), ["c:roundedCorners", "c:chart", "c:spPr"]);
        let chart = space.child("c:chart").unwrap();
        assert_eq!(names(chart), ["c:title", "c:autoTitleDeleted", "c:plotArea", "c:legend", "c:plotVisOnly", "c:dispBlanksAs"]);
        let plot = chart.child("c:plotArea").unwrap();
        assert_eq!(names(plot), ["c:layout", "c:barChart", "c:catAx", "c:valAx"]);
        let bar = plot.child("c:barChart").unwrap();
        assert_eq!(
            names(bar),
            ["c:barDir", "c:grouping", "c:varyColors", "c:ser", "c:ser", "c:ser", "c:dLbls", "c:gapWidth", "c:overlap", "c:axId", "c:axId"]
        );
        assert_eq!(names(bar.child("c:ser").unwrap()), ["c:idx", "c:order", "c:tx", "c:invertIfNegative", "c:cat", "c:val"]);
        let val_ax = plot.child("c:valAx").unwrap();
        assert_eq!(
            names(val_ax),
            [
                "c:axId",
                "c:scaling",
                "c:delete",
                "c:axPos",
                "c:majorGridlines",
                "c:numFmt",
                "c:majorTickMark",
                "c:minorTickMark",
                "c:tickLblPos",
                "c:spPr",
                "c:crossAx",
                "c:crosses",
                "c:crossBetween"
            ]
        );
        let cat_ax = plot.child("c:catAx").unwrap();
        assert_eq!(
            names(cat_ax),
            [
                "c:axId",
                "c:scaling",
                "c:delete",
                "c:axPos",
                "c:numFmt",
                "c:majorTickMark",
                "c:minorTickMark",
                "c:tickLblPos",
                "c:crossAx",
                "c:crosses",
                "c:auto",
                "c:lblAlgn",
                "c:lblOffset",
                "c:noMultiLvlLbl"
            ]
        );
        let line = parse(&ChartSpec { data_labels: true, ..ChartSpec::sample(ChartType::Line) });
        let lc = line.child("c:chart").unwrap().child("c:plotArea").unwrap().child("c:lineChart").unwrap();
        assert_eq!(names(lc), ["c:grouping", "c:varyColors", "c:ser", "c:ser", "c:ser", "c:dLbls", "c:marker", "c:axId", "c:axId"]);
        assert_eq!(names(lc.child("c:ser").unwrap()), ["c:idx", "c:order", "c:tx", "c:marker", "c:cat", "c:val", "c:smooth"]);
        let sc = parse(&ChartSpec::sample(ChartType::Scatter));
        let sc = sc.child("c:chart").unwrap().child("c:plotArea").unwrap().child("c:scatterChart").unwrap();
        assert_eq!(names(sc.child("c:ser").unwrap()), ["c:idx", "c:order", "c:tx", "c:spPr", "c:marker", "c:xVal", "c:yVal", "c:smooth"]);
        let dn = parse(&ChartSpec::sample(ChartType::Doughnut));
        let dn = dn.child("c:chart").unwrap().child("c:plotArea").unwrap();
        assert_eq!(names(dn), ["c:layout", "c:doughnutChart"]);
        assert_eq!(names(dn.child("c:doughnutChart").unwrap()), ["c:varyColors", "c:ser", "c:firstSliceAng", "c:holeSize"]);
    }
}
