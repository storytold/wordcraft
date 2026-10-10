//! Charts (ECMA-376 Part 1 §21.2): the cached data of a `c:chartSpace` drawn as items in points.
//!
//! Bar and column (clustered, stacked or 100% stacked), line, area, pie, doughnut and scatter
//! charts are drawn from the first supported chart type of the plot area. Other chart types draw
//! nothing. Text widths are estimated ([`CHAR_W`] em a character), so label places are approximate.

use std::f64::consts::{FRAC_PI_2, TAU};

use wordcraft_doc::graphic::{GraphicItem, PathSeg, TextAlign};
use wordcraft_doc::para::ShapeKind;
use wordcraft_doc::props::Rgb;

use super::drawing_color::{fill_colour, slot};
use crate::units::{int, measure};
use crate::xml::El;

/// Most series read from one chart.
const MAX_SERIES: usize = 256;
/// Most points read from one series.
const MAX_POINTS: usize = 10_000;
/// Most points read from one chart, all series together.
const MAX_CELLS: usize = 100_000;
/// Longest series name or category label kept.
const MAX_LABEL: usize = 200;
/// Most axis ticks drawn.
const MAX_TICKS: usize = 20;
/// Default text size, points.
const FONT: f32 = 9.0;
/// Average character width estimate, em (charts and diagrams measure text before shaping).
pub(super) const CHAR_W: f32 = 0.55;
/// Widest share of the chart a legend entry's text takes.
const LEGEND_SHARE: f32 = 0.4;
/// Most rows of a legend above or below the plot.
const MAX_LEGEND_ROWS: usize = 3;
const TITLE_SIZE: f32 = 14.0;
/// Space between the frame and the content, points.
const PAD: f32 = 6.0;
/// Line height as a multiple of the font size.
const LINE: f32 = 1.3;
/// Gap between a label and its axis, points.
const GAP: f32 = 4.0;
/// Default width of a line series, points (Word's 2.25 pt).
const LINE_W: f32 = 2.25;
const TEXT: Rgb = Rgb(0x59, 0x59, 0x59);
const GRID: Rgb = Rgb(0xD9, 0xD9, 0xD9);
const AXIS: Rgb = Rgb(0xBF, 0xBF, 0xBF);

/// Items for a chart part, sized `w` x `h` points. Empty when nothing supported is in it.
pub(crate) fn chart_items(space: &El, theme: &[Rgb], w: f32, h: f32) -> Vec<GraphicItem> {
    let mut out = Vec::new();
    if !(w.is_finite() && h.is_finite() && w > 0.0 && h > 0.0) {
        return out;
    }
    let Some(chart) = Chart::parse(space, theme) else { return out };
    backdrop(&mut out, space, theme, w, h);
    let mut area = Rect { x: PAD, y: PAD, w: (w - 2.0 * PAD).max(1.0), h: (h - 2.0 * PAD).max(1.0) };
    if let Some(t) = &chart.title {
        let lh = TITLE_SIZE * LINE;
        text(&mut out, Rect { x: area.x, y: area.y, w: area.w, h: lh }, t, TITLE_SIZE, TextAlign::Center, true);
        area.y += lh + PAD;
        area.h = (area.h - lh - PAD).max(1.0);
    }
    if let Some(pos) = chart.legend {
        // Word lists bar series in the order they are drawn: reversed for clustered horizontal bars
        // and for stacked columns with a vertical legend.
        let vertical = matches!(pos, Pos::Left | Pos::Right);
        let reverse = chart.kind == Kind::Bar && if chart.horizontal { !chart.stacked() } else { chart.stacked() && vertical };
        let mut entries = chart.legend_entries();
        if reverse {
            entries.reverse();
        }
        area = legend(&mut out, &entries, pos, area, chart.text_size);
    }
    match chart.kind {
        Kind::Pie | Kind::Doughnut => pie(&mut out, &chart, area),
        _ => cartesian(&mut out, &chart, area),
    }
    out
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    Bar,
    Line,
    Area,
    Pie,
    Doughnut,
    Scatter,
}

impl Kind {
    fn of(name: &str) -> Option<Kind> {
        Some(match name {
            "c:barChart" => Kind::Bar,
            "c:lineChart" => Kind::Line,
            "c:areaChart" => Kind::Area,
            "c:pieChart" => Kind::Pie,
            "c:doughnutChart" => Kind::Doughnut,
            "c:scatterChart" => Kind::Scatter,
            _ => return None,
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Grouping {
    Clustered,
    Stacked,
    Percent,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Pos {
    Right,
    Left,
    Top,
    Bottom,
}

/// One value axis (or category axis) as the file states it.
#[derive(Default)]
struct Axis {
    deleted: bool,
    grid: bool,
    line: bool,
    /// `c:crossBetween val="midCat"`: categories sit on the tick marks, not between them.
    mid: bool,
    min: Option<f64>,
    max: Option<f64>,
    /// Number format of the tick labels, when not linked to the source data.
    fmt: Option<String>,
}

struct Series {
    name: String,
    color: Option<Rgb>,
    line_w: f32,
    /// Whether the series is drawn as a line (scatter series without one get markers).
    line: bool,
    values: Vec<Option<f64>>,
    /// Scatter x values (index + 1 when the file has none).
    xs: Vec<Option<f64>>,
    /// Per-point colours from `c:dPt`.
    points: Vec<Option<Rgb>>,
    fmt: Option<String>,
    /// Format code of the scatter x values' cache.
    x_fmt: Option<String>,
}

struct Chart {
    kind: Kind,
    horizontal: bool,
    grouping: Grouping,
    /// Gap between groups as a fraction of a bar's width.
    gap: f32,
    /// Overlap of clustered bars, -1..1 (1 = all on top of each other).
    overlap: f32,
    /// Doughnut hole, fraction of the outer radius.
    hole: f32,
    first_angle: f64,
    vary: bool,
    /// Number of categories (scatter: points per series at most).
    n: usize,
    cats: Vec<String>,
    series: Vec<Series>,
    accents: Vec<Rgb>,
    title: Option<String>,
    legend: Option<Pos>,
    /// Category axis (scatter: the x axis).
    cat: Axis,
    val: Axis,
    text_size: f32,
    val_fmt: String,
    /// Serial days count from 1904 (`c:date1904`), not 1900.
    date1904: bool,
    /// Outline of the plot area, when the file draws one.
    plot_border: Option<Rgb>,
}

impl Chart {
    fn parse(space: &El, theme: &[Rgb]) -> Option<Chart> {
        let chart = space.child("c:chart")?;
        let plot = chart.child("c:plotArea")?;
        let ty = plot.els().find(|e| Kind::of(&e.name).is_some())?;
        let kind = Kind::of(&ty.name)?;
        let scatter = kind == Kind::Scatter;
        let date1904 = space.child("c:date1904").is_some_and(|d| d.attr("val").is_none_or(truthy));
        let mut series = read_series(ty, scatter, theme);
        if series.is_empty() {
            return None;
        }
        let cats = ty.children("c:ser").find_map(|s| s.child("c:cat")).map(|c| labels(Some(c), MAX_POINTS, date1904)).unwrap_or_default();
        let n = if matches!(kind, Kind::Pie | Kind::Doughnut) {
            series.first().map_or(0, |s| s.values.len()).max(cats.len())
        } else {
            series.iter().map(|s| s.values.len()).max().unwrap_or(0).max(cats.len())
        };
        // Every series is padded to `n` points, so keep only as many as the cell budget holds.
        if let Some(keep) = MAX_CELLS.checked_div(n) {
            series.truncate(keep.max(1));
        }
        let grouping = match ty.child("c:grouping").and_then(|g| g.attr("val")) {
            Some("stacked") if matches!(kind, Kind::Bar | Kind::Area | Kind::Line) => Grouping::Stacked,
            Some("percentStacked") if matches!(kind, Kind::Bar | Kind::Area | Kind::Line) => Grouping::Percent,
            _ => Grouping::Clustered,
        };
        let mut cats = cats;
        cats.resize(n, String::new());
        for s in &mut series {
            s.values.resize(n, None);
            s.xs.resize(n, None);
        }
        if grouping == Grouping::Percent {
            to_percent(&mut series, n);
        }
        let (cat, val) = chart_axes(plot, scatter);
        let val_fmt = if grouping == Grouping::Percent {
            "0%".to_string()
        } else {
            val.fmt.clone().or_else(|| series.iter().find_map(|s| s.fmt.clone())).unwrap_or_else(|| "General".to_string())
        };
        let num = |n: &str| ty.child(n).and_then(|g| g.attr("val")).and_then(int);
        Some(Chart {
            kind,
            horizontal: kind == Kind::Bar && ty.child("c:barDir").and_then(|d| d.attr("val")) == Some("bar"),
            grouping,
            gap: num("c:gapWidth").unwrap_or(150).clamp(0, 500) as f32 / 100.0,
            overlap: num("c:overlap").unwrap_or(0).clamp(-100, 100) as f32 / 100.0,
            hole: num("c:holeSize").unwrap_or(50).clamp(10, 90) as f32 / 100.0,
            first_angle: num("c:firstSliceAng").unwrap_or(0).clamp(0, 360) as f64,
            vary: matches!(kind, Kind::Pie | Kind::Doughnut) && ty.child("c:varyColors").and_then(|v| v.attr("val")).is_none_or(truthy),
            n,
            cats,
            title: title_of(chart, &series),
            series,
            accents: (4..10).map(|i| slot(theme, i)).collect(),
            legend: legend_pos(chart),
            cat,
            val,
            text_size: text_size(space),
            val_fmt,
            date1904,
            plot_border: plot.child("c:spPr").and_then(|p| p.child("a:ln")).and_then(|l| fill_colour(l, theme)),
        })
    }

    fn stacked(&self) -> bool {
        self.grouping != Grouping::Clustered
    }

    fn accent(&self, i: usize) -> Rgb {
        self.accents.get(i % 6).copied().unwrap_or(Rgb::BLACK)
    }

    fn series_colour(&self, j: usize) -> Rgb {
        self.series.get(j).and_then(|s| s.color).unwrap_or_else(|| self.accent(j))
    }

    /// Colour of point `i` of series `j`: its `c:dPt` override, else the varied accent (pie) or
    /// the series colour.
    fn point_colour(&self, j: usize, i: usize) -> Rgb {
        if let Some(Some(c)) = self.series.get(j).and_then(|s| s.points.get(i)) {
            return *c;
        }
        if self.vary { self.accent(i) } else { self.series_colour(j) }
    }

    fn legend_entries(&self) -> Vec<(String, Rgb)> {
        if matches!(self.kind, Kind::Pie | Kind::Doughnut) {
            return self
                .cats
                .iter()
                .enumerate()
                .map(|(i, c)| (if c.is_empty() { (i + 1).to_string() } else { c.clone() }, self.point_colour(0, i)))
                .collect();
        }
        self.series
            .iter()
            .enumerate()
            .map(|(j, s)| (if s.name.is_empty() { format!("Series {}", j + 1) } else { s.name.clone() }, self.series_colour(j)))
            .collect()
    }
}

/// The series of a chart type element, within the chart's cell budget.
fn read_series(ty: &El, scatter: bool, theme: &[Rgb]) -> Vec<Series> {
    let mut budget = MAX_CELLS;
    let mut series = Vec::new();
    for ser in ty.children("c:ser").take(MAX_SERIES) {
        let s = Series::parse(ser, scatter, theme, budget.min(MAX_POINTS));
        budget = budget.saturating_sub(s.values.len().max(s.xs.len()));
        series.push(s);
    }
    series
}

/// Values as their share of each category's total (100% stacked charts).
fn to_percent(series: &mut [Series], n: usize) {
    for i in 0..n {
        let total: f64 = series.iter().map(|s| s.values.get(i).copied().flatten().map_or(0.0, f64::abs)).sum();
        for s in series.iter_mut() {
            if let Some(Some(v)) = s.values.get_mut(i) {
                *v = if total > 0.0 { *v / total } else { 0.0 };
            }
        }
    }
}

/// The category axis (scatter: the horizontal value axis) and the value axis of a plot area.
fn chart_axes(plot: &El, scatter: bool) -> (Axis, Axis) {
    let vals_ax: Vec<&El> = plot.children("c:valAx").collect();
    let horiz_pos = |a: &&El| matches!(a.child("c:axPos").and_then(|p| p.attr("val")), Some("b" | "t"));
    let (val_el, x_el) = if scatter {
        let x = vals_ax.iter().copied().find(horiz_pos).or_else(|| vals_ax.first().copied());
        let y = vals_ax.iter().copied().find(|a| !horiz_pos(a)).or_else(|| vals_ax.iter().copied().find(|a| Some(*a) != x));
        (y, x)
    } else {
        (vals_ax.first().copied(), None)
    };
    let cat_el = plot.els().find(|e| matches!(e.name.as_str(), "c:catAx" | "c:dateAx"));
    (axis(if scatter { x_el } else { cat_el }), axis(val_el))
}

/// The chart's title. An empty `c:title` shows the series name (Word's automatic title of a
/// one-series chart) unless the automatic title is deleted.
fn title_of(chart: &El, series: &[Series]) -> Option<String> {
    let auto_deleted = chart.child("c:autoTitleDeleted").and_then(|a| a.attr("val")).is_some_and(truthy);
    let auto = if !auto_deleted && series.len() == 1 { series.first().map(|s| s.name.clone()).filter(|s| !s.is_empty()) } else { None };
    chart.child("c:title").and_then(|t| title_text(t).or(auto))
}

fn legend_pos(chart: &El) -> Option<Pos> {
    chart.child("c:legend").map(|l| match l.child("c:legendPos").and_then(|p| p.attr("val")).unwrap_or("r") {
        "l" => Pos::Left,
        "t" | "tr" => Pos::Top,
        "b" => Pos::Bottom,
        _ => Pos::Right,
    })
}

/// The chart's text size (its `c:txPr` default run size), points.
fn text_size(space: &El) -> f32 {
    space
        .child("c:txPr")
        .and_then(|t| t.find("a:defRPr"))
        .and_then(|d| d.attr("sz"))
        .and_then(int)
        .map_or(FONT, |v| (v as f32 / 100.0).clamp(6.0, 24.0))
}

impl Series {
    fn parse(ser: &El, scatter: bool, theme: &[Rgb], cap: usize) -> Series {
        let name = ser.child("c:tx").and_then(|t| t.find("c:v")).map(|v| label(v.text().trim())).unwrap_or_default();
        let sp = ser.child("c:spPr");
        let ln = sp.and_then(|p| p.child("a:ln"));
        let marker = ser.child("c:marker").and_then(|m| m.child("c:spPr")).and_then(|p| fill_colour(p, theme));
        let color = sp.and_then(|p| fill_colour(p, theme)).or_else(|| ln.and_then(|l| fill_colour(l, theme))).or(marker);
        let line_w = ln.and_then(|l| l.attr("w")).and_then(|v| measure(v, 12_700.0)).map_or(LINE_W, |w| w.clamp(0.0, 20.0));
        let line = ln.is_none_or(|l| l.child("a:noFill").is_none());
        let (values, fmt) = numbers(ser.child(if scatter { "c:yVal" } else { "c:val" }), cap);
        let (mut xs, x_fmt) = if scatter { numbers(ser.child("c:xVal"), cap) } else { (Vec::new(), None) };
        if scatter && xs.is_empty() {
            xs = (1..=values.len()).map(|i| Some(i as f64)).collect();
        }
        let mut points = vec![None; values.len()];
        for dpt in ser.children("c:dPt") {
            let i = dpt.child("c:idx").and_then(|e| e.attr("val")).and_then(int).and_then(|i| usize::try_from(i).ok());
            let c = dpt.child("c:spPr").and_then(|p| fill_colour(p, theme));
            if let Some(slot) = i.and_then(|i| points.get_mut(i)) {
                *slot = c;
            }
        }
        Series { name, color, line_w, line, values, xs, points, fmt, x_fmt }
    }
}

/// Category labels of a `c:cat` (string or number cache), capped. Numbers show in the cache's
/// format code (dates included); text and `General` numbers as they are stored.
fn labels(el: Option<&El>, cap: usize, date1904: bool) -> Vec<String> {
    let Some(el) = el else { return Vec::new() };
    let Some(cache) = el.find("c:strCache").or_else(|| el.find("c:numCache")).or_else(|| el.find("c:strLit")).or_else(|| el.find("c:numLit")) else {
        return Vec::new();
    };
    let code = cache.child("c:formatCode").map(|f| f.text()).filter(|f| !f.is_empty() && !f.eq_ignore_ascii_case("General"));
    pts(cache, cap)
        .into_iter()
        .map(|p| {
            let text = p.unwrap_or_default();
            match (code.as_deref(), text.trim().parse::<f64>()) {
                (Some(code), Ok(v)) if v.is_finite() => label(&category_text(v, code, 1.0, date1904)),
                _ => label(&text),
            }
        })
        .collect()
}

/// A number in its format code: a date or time for date codes (serial days, from 1904 when
/// `date1904`), else as [`format_value`] with tick step `step`.
fn category_text(v: f64, code: &str, step: f64, date1904: bool) -> String {
    match excel_stamp(v, date1904) {
        Some(t) if is_date_code(code) => date_text(t, code),
        _ => format_value(v, code, step),
    }
}

/// Values of a `c:val`-style element (number cache or literal), capped; with its format code.
fn numbers(el: Option<&El>, cap: usize) -> (Vec<Option<f64>>, Option<String>) {
    let Some(cache) = el.and_then(|e| e.find("c:numCache").or_else(|| e.find("c:numLit"))) else {
        return (Vec::new(), None);
    };
    let fmt = cache.child("c:formatCode").map(|f| f.text()).filter(|f| !f.is_empty());
    let vals = pts(cache, cap).into_iter().map(|p| p.and_then(|t| t.trim().parse::<f64>().ok()).filter(|v| v.is_finite())).collect();
    (vals, fmt)
}

/// The `c:pt` texts of a cache by index. Indexes and counts from the file are capped.
fn pts(cache: &El, cap: usize) -> Vec<Option<String>> {
    let declared = cache.child("c:ptCount").and_then(|c| c.attr("val")).and_then(int).unwrap_or(0).clamp(0, cap as i64) as usize;
    let mut out: Vec<Option<String>> = vec![None; declared];
    for p in cache.children("c:pt") {
        let Some(i) = p.attr("idx").and_then(int).and_then(|i| usize::try_from(i).ok()) else { continue };
        if i >= cap {
            continue;
        }
        if out.len() <= i {
            out.resize(i + 1, None);
        }
        if let Some(slot) = out.get_mut(i) {
            *slot = p.child("c:v").map(|v| v.text());
        }
    }
    out
}

fn axis(a: Option<&El>) -> Axis {
    let Some(a) = a else { return Axis { deleted: true, ..Default::default() } };
    let scaling = a.child("c:scaling");
    let num =
        |n: &str| scaling.and_then(|s| s.child(n)).and_then(|e| e.attr("val")).and_then(|v| v.trim().parse::<f64>().ok()).filter(|v| v.is_finite());
    Axis {
        deleted: a.child("c:delete").and_then(|d| d.attr("val")).is_some_and(truthy),
        grid: a.child("c:majorGridlines").is_some(),
        line: a.child("c:spPr").and_then(|p| p.child("a:ln")).and_then(|l| l.child("a:noFill")).is_none(),
        mid: a.child("c:crossBetween").and_then(|c| c.attr("val")) == Some("midCat"),
        min: num("c:min"),
        max: num("c:max"),
        fmt: a.child("c:numFmt").filter(|f| f.attr("sourceLinked") != Some("1")).and_then(|f| f.attr("formatCode")).map(str::to_string),
    }
}

fn title_text(t: &El) -> Option<String> {
    let tx = t.child("c:tx")?;
    let s = match tx.child("c:rich") {
        Some(rich) => {
            let mut s = String::new();
            rich_into(rich, &mut s);
            s
        }
        None => tx.find("c:v").map(|v| v.text()).unwrap_or_default(),
    };
    let s = label(s.trim());
    (!s.is_empty()).then_some(s)
}

/// The `a:t` runs under an element, in order.
fn rich_into(e: &El, s: &mut String) {
    for c in e.els() {
        if c.name == "a:t" {
            s.push_str(&c.text());
        } else {
            rich_into(c, s);
        }
    }
}

fn truthy(v: &str) -> bool {
    matches!(v.trim(), "1" | "true")
}

fn label(s: &str) -> String {
    s.chars().take(MAX_LABEL).collect()
}

/// Chart area: the chart space's fill and border (white and borderless by default).
fn backdrop(out: &mut Vec<GraphicItem>, space: &El, theme: &[Rgb], w: f32, h: f32) {
    let sp = space.child("c:spPr");
    let fill = match sp {
        Some(p) if p.child("a:noFill").is_some() => None,
        Some(p) => fill_colour(p, theme).or(Some(Rgb::WHITE)),
        None => Some(Rgb::WHITE),
    };
    let ln = sp.and_then(|p| p.child("a:ln"));
    let stroke = ln.and_then(|l| fill_colour(l, theme));
    let stroke_width = ln.and_then(|l| l.attr("w")).and_then(|v| measure(v, 12_700.0)).unwrap_or(0.75).clamp(0.0, 20.0);
    if fill.is_some() || stroke.is_some() {
        out.push(GraphicItem::Shape { rect: [0.0, 0.0, w, h], kind: ShapeKind::Rectangle, fill, stroke, stroke_width });
    }
}

/// An axis scale: `lo` to `hi` in steps of `step`.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Scale {
    lo: f64,
    hi: f64,
    step: f64,
}

impl Scale {
    fn ticks(&self) -> Vec<f64> {
        let n = (((self.hi - self.lo) / self.step + 1e-9).floor().max(0.0) as usize).min(MAX_TICKS);
        (0..=n).map(|k| self.lo + k as f64 * self.step).filter(|t| *t <= self.hi + 1e-9).collect()
    }
}

/// A scale for data `dlo..dhi` (zero included when `zero`), as Excel and Word pick it: an end
/// away from zero sits 5% of the data span beyond the data (an end at zero stays there), rounded
/// out to a step of 1, 2, 2.5, 5 or 10 times a power of ten (about seven steps). `min`/`max`
/// from the file replace the ends.
fn nice_scale(dlo: f64, dhi: f64, zero: bool, min: Option<f64>, max: Option<f64>) -> Scale {
    let (dlo, dhi) = if dhi > dlo { (dlo, dhi) } else { (dlo, dlo + 1.0) };
    let span = dhi - dlo;
    // With zero, the ends reach it: all-negative data still starts its bars at zero.
    let (ends_lo, ends_hi) = if zero { (dlo.min(0.0), dhi.max(0.0)) } else { (dlo, dhi) };
    let lo = min.unwrap_or(if ends_lo < 0.0 { ends_lo - 0.05 * span } else { ends_lo });
    let hi = max.unwrap_or(if ends_hi > 0.0 { ends_hi + 0.05 * span } else { ends_hi });
    if !(hi > lo && hi.is_finite() && lo.is_finite()) {
        return Scale { lo: 0.0, hi: 1.0, step: 0.2 };
    }
    let raw = (hi - lo) / 7.5;
    let mag = 10f64.powf(raw.log10().floor());
    let step = [1.0, 2.0, 2.5, 5.0, 10.0].iter().map(|m| m * mag).find(|s| *s >= raw).unwrap_or(10.0 * mag);
    let lo_r = if min.is_some() { lo } else { ((lo / step) + 1e-9).floor() * step };
    let hi_r = if max.is_some() { hi } else { ((hi / step) - 1e-9).ceil() * step };
    if !(step.is_finite() && step > 0.0 && lo_r.is_finite() && hi_r.is_finite() && hi_r > lo_r) {
        return Scale { lo: 0.0, hi: 1.0, step: 0.2 };
    }
    Scale { lo: lo_r, hi: hi_r, step }
}

/// A piece of a format section: literal text, a number character (`#`, `0`, `,`, `.`), or the
/// exponent of a scientific format (`E+` when true, `E-` when false).
enum Tok {
    Lit(String),
    Num(char),
    Exp(bool),
}

/// The sections of a format code (positive; negative; zero; text), split outside quotes.
fn sections(code: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let (mut start, mut quoted, mut escaped) = (0, false, false);
    for (i, c) in code.char_indices() {
        match c {
            _ if escaped => escaped = false,
            '\\' if !quoted => escaped = true,
            '"' => quoted = !quoted,
            ';' if !quoted => {
                out.push(code.get(start..i).unwrap_or(""));
                start = i + 1;
            }
            _ => {}
        }
    }
    out.push(code.get(start..).unwrap_or(""));
    out
}

/// The first section of a format code.
fn first_section(code: &str) -> &str {
    sections(code).first().copied().unwrap_or("")
}

/// The pieces of one format section. Quoted text, escapes and `[$€-407]` currency symbols are
/// literal; other brackets (`[Red]`) are dropped; `_x` (room for `x`) is a space; `*x` is dropped.
fn format_toks(section: &str) -> Vec<Tok> {
    let mut toks = Vec::new();
    let mut chars = section.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '"' => toks.push(Tok::Lit(chars.by_ref().take_while(|q| *q != '"').collect())),
            '[' => {
                let inner: String = chars.by_ref().take_while(|q| *q != ']').collect();
                if let Some(sym) = currency(&inner) {
                    toks.push(Tok::Lit(sym.to_string()));
                }
            }
            '\\' => toks.extend(chars.next().map(|n| Tok::Lit(n.to_string()))),
            '_' => {
                chars.next();
                toks.push(Tok::Lit(" ".to_string()));
            }
            '*' => {
                chars.next();
            }
            'E' | 'e' if matches!(chars.peek(), Some('+' | '-')) => toks.push(Tok::Exp(chars.next() == Some('+'))),
            '#' | '0' | ',' | '.' => toks.push(Tok::Num(c)),
            c => toks.push(Tok::Lit(c.to_string())),
        }
    }
    toks
}

/// The symbol of a `$€-407` bracket content (`[$€-407]`), if it is one.
fn currency(inner: &str) -> Option<&str> {
    inner.strip_prefix('$').map(|rest| rest.split('-').next().unwrap_or(""))
}

/// Tick text for `v` in a number format code (`#,##0`, `0.0`, `0%`, `0.00E+00`, and literal text
/// such as `"€"0.00` or `#,##0.00 [$€-407]`). A negative number takes the second section when
/// there is one, which shows it without a minus (`#,##0;(#,##0)`). `General` shows as many
/// decimals as the step needs.
fn format_value(v: f64, code: &str, step: f64) -> String {
    let v = if v.abs() < step * 1e-9 { 0.0 } else { v };
    let secs = sections(code);
    match secs.get(1) {
        Some(neg) if v < 0.0 => format_section(-v, neg, step),
        _ => format_section(v, secs.first().copied().unwrap_or(""), step),
    }
}

/// `v` in one section of a format code.
fn format_section(v: f64, section: &str, step: f64) -> String {
    let toks = format_toks(section);
    let is_num = |t: &Tok| matches!(t, Tok::Num(_) | Tok::Exp(_));
    let lit = |r: &[Tok]| -> String {
        r.iter()
            .filter_map(|t| match t {
                Tok::Lit(l) => Some(l.as_str()),
                _ => None,
            })
            .collect()
    };
    let (Some(s), Some(e)) = (toks.iter().position(is_num), toks.iter().rposition(is_num)) else { return general(v, step) };
    let body = toks.get(s..=e).unwrap_or(&[]);
    let (pre, suf) = (lit(toks.get(..s).unwrap_or(&[])), lit(toks.get(e + 1..).unwrap_or(&[])));
    let digits = |r: &[Tok]| -> String { r.iter().filter_map(|t| if let Tok::Num(c) = t { Some(*c) } else { None }).collect() };
    if let Some(k) = body.iter().position(|t| matches!(t, Tok::Exp(_))) {
        let plus = matches!(body.get(k), Some(Tok::Exp(true)));
        let mantissa = digits(body.get(..k).unwrap_or(&[]));
        let exp_digits = digits(body.get(k + 1..).unwrap_or(&[])).len();
        return format!("{pre}{}{suf}", scientific(v, decimals(&mantissa), exp_digits, plus));
    }
    let body = digits(body);
    let pct = pre.contains('%') || suf.contains('%');
    let v = if pct { v * 100.0 } else { v };
    let digits = format!("{:.*}", decimals(&body), v.abs());
    let (ip, fp) = match digits.split_once('.') {
        Some((a, b)) => (a.to_string(), format!(".{b}")),
        None => (digits.clone(), String::new()),
    };
    let mut grouped = String::new();
    for (k, ch) in ip.chars().enumerate() {
        if body.contains(',') && k > 0 && (ip.len() - k) % 3 == 0 {
            grouped.push(',');
        }
        grouped.push(ch);
    }
    let sign = if v < 0.0 && digits.chars().any(|c| c != '0' && c != '.') { "-" } else { "" };
    format!("{sign}{pre}{grouped}{fp}{suf}")
}

/// Decimal places of a number body (`#,##0.00` has 2).
fn decimals(body: &str) -> usize {
    body.split_once('.').map_or(0, |(_, d)| d.chars().filter(|c| matches!(c, '0' | '#')).count()).min(12)
}

/// `v` with no number format (`General`): as many decimals as the tick step needs.
fn general(v: f64, step: f64) -> String {
    let mut d = 0;
    while d < 6 && (step * 10f64.powi(d)).fract().abs() > 1e-9 {
        d += 1;
    }
    let s = format!("{:.*}", d as usize, v);
    if s.trim_start_matches(['-', '0', '.']).is_empty() { s.trim_start_matches('-').to_string() } else { s }
}

/// `v` in scientific notation: `decimals` mantissa decimals, at least `exp_digits` exponent
/// digits, and a `+` on positive exponents when `plus` (`0.00E+00`: 1.23E+04).
fn scientific(v: f64, decimals: usize, exp_digits: usize, plus: bool) -> String {
    let s = format!("{:.*e}", decimals, v.abs());
    let (mantissa, exp) = s.split_once('e').unwrap_or((s.as_str(), "0"));
    let exp: i32 = if v == 0.0 { 0 } else { exp.parse().unwrap_or(0) };
    let sign = if exp < 0 {
        "-"
    } else if plus {
        "+"
    } else {
        ""
    };
    let minus = if v < 0.0 { "-" } else { "" };
    format!("{minus}{mantissa}E{sign}{:0w$}", exp.unsigned_abs(), w = exp_digits.clamp(1, 4))
}

/// A moment as Excel shows a serial day number.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Stamp {
    y: i64,
    m: u32,
    d: u32,
    /// Day of the week, 0 = Sunday.
    weekday: u32,
    /// Seconds into the day.
    secs: u32,
}

/// The moment of Excel serial day `serial` as Excel shows it, from 0 ("January 0, 1900") to
/// 31 December 9999. In the 1900 system serial 60 is Excel's phantom 29 February 1900, and
/// serials before it count from 31 December 1899; in the 1904 system (`date1904`) serial 0 is
/// 1 January 1904. `None` outside that range.
fn excel_stamp(serial: f64, date1904: bool) -> Option<Stamp> {
    let serial = if date1904 { serial + 1462.0 } else { serial };
    if !(0.0..2_958_466.0).contains(&serial) {
        return None;
    }
    let whole = serial.floor();
    let secs = (((serial - whole) * 86_400.0).round() as u32).min(86_399);
    let day = whole as i64;
    let (y, m, d) = match day {
        0 => (1900, 1, 0),
        60 => (1900, 2, 29),
        1..=59 => civil(day + 1),
        _ => civil(day),
    };
    // Excel's weekdays count serial 1 as a Sunday.
    Some(Stamp { y, m, d, weekday: (day - 1).rem_euclid(7) as u32, secs })
}

/// Calendar date (year, month, day) of day `n` counted from 30 December 1899 (Howard Hinnant's
/// civil-from-days algorithm).
fn civil(n: i64) -> (i64, u32, u32) {
    let z = n - 25_569 + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = (if mp < 10 { mp + 3 } else { mp - 9 }) as u32;
    (yoe + era * 400 + i64::from(m <= 2), m, d)
}

const MONTHS: [&str; 12] = ["January", "February", "March", "April", "May", "June", "July", "August", "September", "October", "November", "December"];
const WEEKDAYS: [&str; 7] = ["Sunday", "Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday"];

/// Whether a format code (outside its quoted and bracketed parts) has date or time fields.
fn is_date_code(code: &str) -> bool {
    format_toks_raw(code).any(|c| matches!(c.to_ascii_lowercase(), 'y' | 'd' | 'm' | 'h' | 's'))
}

/// The characters of a format code's first section outside quotes and brackets.
fn format_toks_raw(code: &str) -> impl Iterator<Item = char> + '_ {
    let mut quoted = false;
    let mut bracket = false;
    first_section(code).chars().filter(move |c| match c {
        '"' => {
            quoted = !quoted;
            false
        }
        '[' if !quoted => {
            bracket = true;
            false
        }
        ']' if bracket => {
            bracket = false;
            false
        }
        _ => !quoted && !bracket,
    })
}

/// A piece of a date format: literal text, a field (`y`, `m`, `d`, `h`, `s`, or `n` for minutes)
/// with its length, or AM/PM (`AM/PM` when true, `A/P` when false).
enum DTok {
    Lit(String),
    Field(char, usize),
    AmPm(bool),
}

/// The pieces of a date format's first section. `m` or `mm` right after an hour or right before
/// a seconds field is minutes.
fn date_toks(code: &str) -> Vec<DTok> {
    let section: Vec<char> = first_section(code).chars().collect();
    let mut toks = Vec::new();
    let mut i = 0;
    while let Some(&c) = section.get(i) {
        let at = i;
        i += 1;
        match c {
            '"' => {
                let lit: String = section.get(i..).unwrap_or(&[]).iter().take_while(|q| **q != '"').collect();
                i += lit.chars().count() + 1;
                toks.push(DTok::Lit(lit));
            }
            '[' => {
                let inner: String = section.get(i..).unwrap_or(&[]).iter().take_while(|q| **q != ']').collect();
                i += inner.chars().count() + 1;
                toks.extend(currency(&inner).map(|s| DTok::Lit(s.to_string())));
            }
            '\\' | '_' | '*' => {
                let next = section.get(i).copied();
                i += 1;
                match (c, next) {
                    ('\\', Some(n)) => toks.push(DTok::Lit(n.to_string())),
                    ('_', _) => toks.push(DTok::Lit(" ".to_string())),
                    _ => {}
                }
            }
            'a' | 'A' if starts_ci(&section, at, "AM/PM") => {
                i += 4;
                toks.push(DTok::AmPm(true));
            }
            'a' | 'A' if starts_ci(&section, at, "A/P") => {
                i += 2;
                toks.push(DTok::AmPm(false));
            }
            _ if matches!(c.to_ascii_lowercase(), 'y' | 'm' | 'd' | 'h' | 's') => {
                let mut n = 1;
                while section.get(i).is_some_and(|q| q.eq_ignore_ascii_case(&c)) {
                    i += 1;
                    n += 1;
                }
                toks.push(DTok::Field(c.to_ascii_lowercase(), n));
            }
            c => toks.push(DTok::Lit(c.to_string())),
        }
    }
    mark_minutes(&mut toks);
    toks
}

/// Whether `chars` from `at` on start with `pat`, ignoring ASCII case.
fn starts_ci(chars: &[char], at: usize, pat: &str) -> bool {
    pat.chars().enumerate().all(|(k, p)| chars.get(at + k).is_some_and(|c| c.eq_ignore_ascii_case(&p)))
}

/// Turns the `m`/`mm` fields that mean minutes (after an hour, or before seconds) into `n`.
fn mark_minutes(toks: &mut [DTok]) {
    let fields: Vec<usize> = toks.iter().enumerate().filter(|(_, t)| matches!(t, DTok::Field(..))).map(|(k, _)| k).collect();
    let kind = |k: Option<&usize>, toks: &[DTok]| match k.and_then(|k| toks.get(*k)) {
        Some(DTok::Field(c, _)) => Some(*c),
        _ => None,
    };
    for (j, k) in fields.iter().enumerate() {
        let after_hour = j.checked_sub(1).and_then(|p| kind(fields.get(p), toks)) == Some('h');
        let before_secs = kind(fields.get(j + 1), toks) == Some('s');
        if let Some(DTok::Field(c @ 'm', n)) = toks.get_mut(*k)
            && *n <= 2
            && (after_hour || before_secs)
        {
            *c = 'n';
        }
    }
}

/// A moment in a date format: day, month, year, weekday (`ddd`, `dddd`), hour, minute and
/// second fields replaced (12-hour with AM/PM); quoted text and currency kept, other brackets
/// dropped.
fn date_text(t: Stamp, code: &str) -> String {
    let toks = date_toks(code);
    let twelve = toks.iter().any(|k| matches!(k, DTok::AmPm(_)));
    let hour = t.secs / 3600;
    let mut out = String::new();
    for tok in &toks {
        match tok {
            DTok::Lit(s) => out.push_str(s),
            DTok::AmPm(long) => out.push_str(match (hour < 12, long) {
                (true, true) => "AM",
                (false, true) => "PM",
                (true, false) => "A",
                (false, false) => "P",
            }),
            DTok::Field(c, n) => out.push_str(&field_text(t, *c, *n, twelve)),
        }
    }
    out
}

/// One date or time field (`c` as in [`DTok::Field`], `n` letters long).
fn field_text(t: Stamp, c: char, n: usize, twelve: bool) -> String {
    let month = MONTHS.get((t.m as usize).clamp(1, 12) - 1).copied().unwrap_or("");
    let weekday = WEEKDAYS.get(t.weekday as usize).copied().unwrap_or("");
    let pad = |v: u32| if n >= 2 { format!("{v:02}") } else { v.to_string() };
    let hour = t.secs / 3600;
    match c {
        'y' if n <= 2 => format!("{:02}", t.y.rem_euclid(100)),
        'y' => format!("{:04}", t.y),
        'd' if n <= 2 => pad(t.d),
        'd' if n == 3 => weekday.get(..3).unwrap_or(weekday).to_string(),
        'd' => weekday.to_string(),
        'm' if n <= 2 => pad(t.m),
        'm' if n == 3 => month.get(..3).unwrap_or(month).to_string(),
        'm' if n == 4 => month.to_string(),
        'm' => month.get(..1).unwrap_or(month).to_string(),
        'h' if twelve => pad(if hour.is_multiple_of(12) { 12 } else { hour % 12 }),
        'h' => pad(hour),
        'n' => pad(t.secs / 60 % 60),
        _ => pad(t.secs % 60),
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct Rect {
    x: f32,
    y: f32,
    w: f32,
    h: f32,
}

impl Rect {
    fn right(self) -> f32 {
        self.x + self.w
    }
    fn bottom(self) -> f32 {
        self.y + self.h
    }
    fn arr(self) -> [f32; 4] {
        [self.x, self.y, self.w, self.h]
    }
}

/// Estimated text width, points: text is placed before it's shaped, at [`CHAR_W`] a character.
pub(super) fn text_w(s: &str, size: f32) -> f32 {
    s.chars().count() as f32 * size * CHAR_W
}

/// `s`, cut short with an ellipsis to fit `width` points by the [`text_w`] estimate.
fn clip_text(s: &str, width: f32, size: f32) -> String {
    let fit = (width / (size * CHAR_W)).floor().max(0.0) as usize;
    if s.chars().count() <= fit {
        return s.to_string();
    }
    let mut out: String = s.chars().take(fit.saturating_sub(1)).collect();
    out.push('…');
    out
}

fn widest(items: &[String], size: f32) -> f32 {
    items.iter().map(|s| text_w(s, size)).fold(0.0, f32::max)
}

fn text(out: &mut Vec<GraphicItem>, rect: Rect, s: &str, size: f32, align: TextAlign, bold: bool) {
    out.push(GraphicItem::Text { rect: rect.arr(), text: s.to_string(), size, color: TEXT, bold, align, font: None });
}

/// Text centred on `cx` with its top at `y`.
fn text_at(out: &mut Vec<GraphicItem>, cx: f32, y: f32, s: &str, size: f32) {
    let w = text_w(s, size) + 4.0;
    text(out, Rect { x: cx - w / 2.0, y, w, h: size * LINE }, s, size, TextAlign::Center, false);
}

fn fill(out: &mut Vec<GraphicItem>, segs: Vec<PathSeg>, color: Rgb) {
    if !segs.is_empty() {
        out.push(GraphicItem::Path { segs, fill: Some(color), stroke: None, stroke_width: 0.0 });
    }
}

fn stroke(out: &mut Vec<GraphicItem>, segs: Vec<PathSeg>, color: Rgb, width: f32) {
    if !segs.is_empty() {
        out.push(GraphicItem::Path { segs, fill: None, stroke: Some(color), stroke_width: width });
    }
}

fn swatch(out: &mut Vec<GraphicItem>, x: f32, y: f32, color: Rgb) {
    out.push(GraphicItem::Shape { rect: [x, y, SWATCH, SWATCH], kind: ShapeKind::Rectangle, fill: Some(color), stroke: None, stroke_width: 0.0 });
}

fn seg(x0: f32, y0: f32, x1: f32, y1: f32) -> Vec<PathSeg> {
    vec![PathSeg::Move(x0, y0), PathSeg::Line(x1, y1)]
}

fn rect_segs(x0: f32, y0: f32, x1: f32, y1: f32) -> Vec<PathSeg> {
    vec![PathSeg::Move(x0, y0), PathSeg::Line(x1, y0), PathSeg::Line(x1, y1), PathSeg::Line(x0, y1), PathSeg::Close]
}

/// Polyline pieces: a gap (`None`) starts a new sub-path.
fn runs(pts: impl Iterator<Item = Option<(f32, f32)>>) -> Vec<PathSeg> {
    let mut segs = Vec::new();
    let mut open = false;
    for p in pts {
        match p {
            Some((x, y)) => {
                segs.push(if open { PathSeg::Line(x, y) } else { PathSeg::Move(x, y) });
                open = true;
            }
            None => open = false,
        }
    }
    segs
}

/// Words of `s` in lines of at most `width` points (a word wider than that keeps its own line).
fn wrap(s: &str, width: f32, size: f32) -> Vec<String> {
    let mut lines: Vec<String> = Vec::new();
    let mut cur = String::new();
    for word in s.split_whitespace() {
        let joined = if cur.is_empty() { word.to_string() } else { format!("{cur} {word}") };
        if cur.is_empty() || text_w(&joined, size) <= width {
            cur = joined;
        } else {
            lines.push(std::mem::take(&mut cur));
            cur = word.to_string();
        }
    }
    if !cur.is_empty() {
        lines.push(cur);
    }
    if lines.is_empty() {
        lines.push(String::new());
    }
    lines.truncate(3);
    lines
}

const SWATCH: f32 = 7.0;
/// Space between legend entries in a row, points.
const SEP: f32 = 10.0;

/// Legend in the given side of `area`; returns the area left for the plot.
fn legend(out: &mut Vec<GraphicItem>, entries: &[(String, Rgb)], pos: Pos, area: Rect, fs: f32) -> Rect {
    if entries.is_empty() {
        return area;
    }
    // Entry texts are cut to a share of the chart, so a long name can't push the legend out.
    let entries: Vec<(String, Rgb)> = entries.iter().map(|(s, c)| (clip_text(s, area.w * LEGEND_SHARE, fs), *c)).collect();
    match pos {
        Pos::Right | Pos::Left => side_legend(out, &entries, pos == Pos::Left, area, fs),
        Pos::Top | Pos::Bottom => row_legend(out, &entries, pos == Pos::Top, area, fs),
    }
}

/// A legend in a column at the left or right of `area`, keeping only the entries that fit above
/// its bottom; returns the area left for the plot.
fn side_legend(out: &mut Vec<GraphicItem>, entries: &[(String, Rgb)], left: bool, area: Rect, fs: f32) -> Rect {
    let lh = fs * LINE;
    let entries = entries.get(..((area.h / lh).floor().max(0.0) as usize).min(entries.len())).unwrap_or(&[]);
    if entries.is_empty() {
        return area;
    }
    let tw = entries.iter().map(|(s, _)| text_w(s, fs)).fold(0.0, f32::max);
    let block_w = SWATCH + GAP + tw;
    let block_h = lh * entries.len() as f32;
    let x = if left { area.x } else { area.right() - block_w };
    let y0 = area.y + ((area.h - block_h) / 2.0).max(0.0);
    for (k, (s, c)) in entries.iter().enumerate() {
        let y = y0 + lh * k as f32;
        swatch(out, x, y + (lh - SWATCH) / 2.0, *c);
        text(out, Rect { x: x + SWATCH + GAP, y, w: tw + 2.0, h: lh }, s, fs, TextAlign::Left, false);
    }
    let used = block_w + PAD;
    let mut area = area;
    area.w = (area.w - used).max(1.0);
    if left {
        area.x += used;
    }
    area
}

/// A legend in centred rows the width of `area` above or below it (at most
/// [`MAX_LEGEND_ROWS`], and half its height); returns the area left for the plot.
fn row_legend(out: &mut Vec<GraphicItem>, entries: &[(String, Rgb)], top: bool, area: Rect, fs: f32) -> Rect {
    let lh = fs * LINE;
    let max_rows = MAX_LEGEND_ROWS.min((area.h / 2.0 / lh).floor().max(1.0) as usize);
    let rows = legend_rows(entries, area.w, fs, max_rows);
    let y0 = if top { area.y } else { area.bottom() - lh * rows.len() as f32 };
    for (r, row) in rows.iter().enumerate() {
        let total = row.iter().map(|(_, _, w)| SWATCH + GAP + w).sum::<f32>() + SEP * row.len().saturating_sub(1) as f32;
        let mut x = area.x + ((area.w - total) / 2.0).max(0.0);
        let y = y0 + lh * r as f32;
        for (s, c, w) in row {
            swatch(out, x, y + (lh - SWATCH) / 2.0, *c);
            text(out, Rect { x: x + SWATCH + GAP, y, w: w + 2.0, h: lh }, s, fs, TextAlign::Left, false);
            x += SWATCH + GAP + w + SEP;
        }
    }
    let used = lh * rows.len() as f32 + PAD;
    let mut area = area;
    area.h = (area.h - used).max(1.0);
    if top {
        area.y += used;
    }
    area
}

/// Legend entries (text, colour, text width) packed into at most `max_rows` rows of `width`
/// points; the entries past the last row are left out.
fn legend_rows(entries: &[(String, Rgb)], width: f32, fs: f32, max_rows: usize) -> Vec<Vec<(String, Rgb, f32)>> {
    let mut rows: Vec<Vec<(String, Rgb, f32)>> = Vec::new();
    let mut row_w = 0.0;
    for (s, c) in entries {
        let w = text_w(s, fs);
        let entry_w = SWATCH + GAP + w;
        let fits = !rows.is_empty() && row_w + SEP + entry_w <= width;
        if fits {
            row_w += SEP + entry_w;
        } else if rows.len() < max_rows {
            rows.push(Vec::new());
            row_w = entry_w;
        } else {
            break;
        }
        if let Some(row) = rows.last_mut() {
            row.push((s.clone(), *c, w));
        }
    }
    rows
}

/// The plot rectangle and the scales that map values and categories onto it.
struct Frame {
    plot: Rect,
    horiz: bool,
    n: usize,
    vs: Scale,
    xs: Option<Scale>,
    mid: bool,
}

impl Frame {
    /// Width of one category band.
    fn band(&self) -> f32 {
        let len = if self.horiz { self.plot.h } else { self.plot.w };
        len / self.n.max(1) as f32
    }

    /// Start of category `i`'s band: its left edge, or for bars its top edge (category 0 at the
    /// bottom).
    fn band_start(&self, i: usize) -> f32 {
        let k = i as f32;
        if self.horiz { self.plot.bottom() - (k + 1.0) * self.band() } else { self.plot.x + k * self.band() }
    }

    /// A value's position along the value direction: y for columns, x for bars.
    fn val(&self, v: f64) -> f32 {
        let t = ((v - self.vs.lo) / (self.vs.hi - self.vs.lo)).clamp(0.0, 1.0) as f32;
        if self.horiz { self.plot.x + t * self.plot.w } else { self.plot.bottom() - t * self.plot.h }
    }

    /// x of category `i` for line and area points.
    fn cat_x(&self, i: usize) -> f32 {
        if !self.mid {
            return self.plot.x + (i as f32 + 0.5) * self.band();
        }
        if self.n <= 1 { self.plot.x + self.plot.w / 2.0 } else { self.plot.x + self.plot.w * i as f32 / (self.n - 1) as f32 }
    }

    /// x of a scatter point.
    fn scat_x(&self, x: f64) -> f32 {
        match self.xs {
            Some(s) => self.plot.x + ((x - s.lo) / (s.hi - s.lo)).clamp(0.0, 1.0) as f32 * self.plot.w,
            None => self.plot.x,
        }
    }
}

/// The value scale and, for scatter charts, the x scale, with their ticks and tick texts.
struct Axes {
    vs: Scale,
    vticks: Vec<(f64, String)>,
    xs: Option<Scale>,
    xticks: Vec<(f64, String)>,
}

impl Axes {
    fn of(c: &Chart) -> Axes {
        let (dlo, dhi) = extent(c);
        let (vmin, vmax) = if c.grouping == Grouping::Percent { (Some(0.0), Some(1.0)) } else { (c.val.min, c.val.max) };
        // Lines and scatters don't need zero for a narrow range (1000..1050 stays on its own scale).
        let zero = !matches!(c.kind, Kind::Line | Kind::Scatter) || !narrow(dlo, dhi);
        let vs = nice_scale(dlo, dhi, zero, vmin, vmax);
        let vticks = vs.ticks().into_iter().map(|t| (t, format_value(t, &c.val_fmt, vs.step))).collect();
        let xs = (c.kind == Kind::Scatter).then(|| {
            let (xl, xh) = x_extent(c);
            nice_scale(xl, xh, false, c.cat.min, c.cat.max)
        });
        // Scatter x values show in the axis's own format, else their cache's (dates included).
        let xfmt = c.cat.fmt.clone().or_else(|| c.series.iter().find_map(|s| s.x_fmt.clone())).unwrap_or_else(|| "General".to_string());
        let xticks = xs.map(|s| s.ticks().into_iter().map(|t| (t, category_text(t, &xfmt, s.step, c.date1904))).collect()).unwrap_or_default();
        Axes { vs, vticks, xs, xticks }
    }
}

/// The room around the plot for axis labels, and the category labels as they'll be drawn.
struct Margins {
    left: f32,
    top: f32,
    bottom: f32,
    /// Category labels in lines (none for scatter charts or a deleted category axis).
    wrapped: Vec<Vec<String>>,
    /// Every `skip`-th category label is shown under a column chart.
    skip: usize,
}

/// The margins the axis labels need around the plot in `area`. Category labels under a column
/// chart wrap to the band; labels that still don't fit show every k-th category.
fn axis_margins(c: &Chart, ax: &Axes, area: Rect) -> Margins {
    let fs = c.text_size;
    let lh = fs * LINE;
    let scatter = c.kind == Kind::Scatter;
    let (cat_on, val_on) = (!c.cat.deleted, !c.val.deleted);
    let left = if c.horizontal {
        if cat_on { widest(&c.cats, fs).min(area.w * 0.4) + GAP } else { 0.0 }
    } else if val_on {
        ax.vticks.iter().map(|(_, s)| text_w(s, fs)).fold(0.0, f32::max) + GAP
    } else {
        0.0
    };
    let band_w = (area.w - left).max(1.0) / c.n as f32;
    let wrapped: Vec<Vec<String>> = if cat_on && !scatter {
        let width = if c.horizontal { left - GAP } else { band_w };
        c.cats.iter().map(|s| wrap(s, width, fs)).collect()
    } else {
        Vec::new()
    };
    let widest_line = wrapped.iter().flatten().map(|s| text_w(s, fs)).fold(0.0, f32::max);
    let skip = ((widest_line / band_w).ceil() as usize).max(1);
    let label_lines = wrapped.iter().map(Vec::len).max().unwrap_or(1) as f32;
    let bottom = if cat_on && !c.horizontal {
        if scatter { lh + GAP } else { label_lines * lh + GAP }
    } else if c.horizontal && val_on {
        lh + GAP
    } else {
        0.0
    };
    let top = if !c.horizontal && val_on { lh / 2.0 } else { 0.0 };
    Margins { left, top, bottom, wrapped, skip }
}

/// A bar, column, line, area or scatter chart in `area`: gridlines, series, axes, then labels.
fn cartesian(out: &mut Vec<GraphicItem>, c: &Chart, area: Rect) {
    if c.n == 0 {
        return;
    }
    let ax = Axes::of(c);
    let m = axis_margins(c, &ax, area);
    let plot = Rect { x: area.x + m.left, y: area.y + m.top, w: area.w - m.left, h: area.h - m.top - m.bottom };
    if plot.w < 1.0 || plot.h < 1.0 {
        return;
    }
    let g = Frame { plot, horiz: c.horizontal, n: c.n, vs: ax.vs, xs: ax.xs, mid: c.val.mid };
    if c.val.grid {
        draw_grid(out, &g, &ax.vticks);
    }
    match c.kind {
        Kind::Bar => bars(out, c, &g),
        Kind::Line => lines(out, c, &g),
        Kind::Area => areas(out, c, &g),
        Kind::Scatter => draw_scatter(out, c, &g),
        Kind::Pie | Kind::Doughnut => {}
    }
    draw_axes(out, c, &g);
    if !c.val.deleted {
        draw_value_labels(out, c, &g, &ax.vticks, Rect { w: m.left, ..area });
    }
    if !c.cat.deleted {
        draw_category_labels(out, c, &g, &ax.xticks, &m, area);
    }
}

/// Major gridlines at the value ticks.
fn draw_grid(out: &mut Vec<GraphicItem>, g: &Frame, ticks: &[(f64, String)]) {
    let plot = g.plot;
    for (t, _) in ticks {
        let p = g.val(*t);
        let segs = if g.horiz { seg(p, plot.y, p, plot.bottom()) } else { seg(plot.x, p, plot.right(), p) };
        stroke(out, segs, GRID, 0.75);
    }
}

/// The category axis line, the plot area's outline and the value axis line, as the file shows them.
fn draw_axes(out: &mut Vec<GraphicItem>, c: &Chart, g: &Frame) {
    let plot = g.plot;
    if !c.cat.deleted && c.cat.line {
        let segs = if g.horiz { seg(plot.x, plot.y, plot.x, plot.bottom()) } else { seg(plot.x, plot.bottom(), plot.right(), plot.bottom()) };
        stroke(out, segs, AXIS, 0.75);
    }
    if let Some(color) = c.plot_border {
        stroke(out, rect_segs(plot.x, plot.y, plot.right(), plot.bottom()), color, 0.75);
    }
    if !c.val.deleted && c.val.line {
        let segs = if g.horiz { seg(plot.x, plot.bottom(), plot.right(), plot.bottom()) } else { seg(plot.x, plot.y, plot.x, plot.bottom()) };
        stroke(out, segs, AXIS, 0.75);
    }
}

/// Value tick labels: under the plot for bars, else right-aligned in `margin` (the area's left
/// margin).
fn draw_value_labels(out: &mut Vec<GraphicItem>, c: &Chart, g: &Frame, ticks: &[(f64, String)], margin: Rect) {
    let (fs, lh) = (c.text_size, c.text_size * LINE);
    for (t, s) in ticks {
        let p = g.val(*t);
        if g.horiz {
            text_at(out, p, g.plot.bottom() + GAP / 2.0, s, fs);
        } else {
            text(out, Rect { x: margin.x, y: p - lh / 2.0, w: margin.w - GAP, h: lh }, s, fs, TextAlign::Right, false);
        }
    }
}

/// Category labels: scatter x ticks under the plot, bar labels in the left margin, column
/// labels under their bands (every `m.skip`-th).
fn draw_category_labels(out: &mut Vec<GraphicItem>, c: &Chart, g: &Frame, xticks: &[(f64, String)], m: &Margins, area: Rect) {
    let (fs, lh) = (c.text_size, c.text_size * LINE);
    let plot = g.plot;
    if c.kind == Kind::Scatter {
        for (t, s) in xticks {
            text_at(out, g.scat_x(*t), plot.bottom() + GAP / 2.0, s, fs);
        }
    } else if g.horiz {
        for (i, lines) in m.wrapped.iter().enumerate() {
            let top = g.band_start(i) + g.band() / 2.0 - lines.len() as f32 * lh / 2.0;
            for (k, s) in lines.iter().enumerate() {
                text(out, Rect { x: area.x, y: top + k as f32 * lh, w: m.left - GAP, h: lh }, s, fs, TextAlign::Right, false);
            }
        }
    } else {
        let band = g.band();
        for (i, lines) in m.wrapped.iter().enumerate().filter(|(i, _)| i % m.skip == 0) {
            for (k, s) in lines.iter().enumerate() {
                let y = plot.bottom() + GAP / 2.0 + k as f32 * lh;
                text(out, Rect { x: g.cat_x(i) - band / 2.0, y, w: band, h: lh }, s, fs, TextAlign::Center, false);
            }
        }
    }
}

/// Whether data `lo..hi` is a narrow band away from zero: its span is under a sixth of the end
/// farther from zero (the mirror image for negative data).
fn narrow(lo: f64, hi: f64) -> bool {
    if lo > 0.0 {
        (hi - lo) / hi < 1.0 / 6.0
    } else if hi < 0.0 {
        (hi - lo) / lo.abs() < 1.0 / 6.0
    } else {
        false
    }
}

/// Lowest and highest value of the plot; stacked series add up per category.
fn extent(c: &Chart) -> (f64, f64) {
    let mut pts: Vec<f64> = Vec::new();
    if c.stacked() {
        for i in 0..c.n {
            let (mut p, mut q) = (0.0, 0.0);
            for s in &c.series {
                match s.values.get(i).copied().flatten() {
                    Some(v) if v >= 0.0 => p += v,
                    Some(v) => q += v,
                    None => {}
                }
            }
            pts.push(p);
            pts.push(q);
        }
    } else {
        pts.extend(c.series.iter().flat_map(|s| s.values.iter().flatten().copied()));
    }
    let lo = pts.iter().copied().fold(f64::INFINITY, f64::min);
    let hi = pts.iter().copied().fold(f64::NEG_INFINITY, f64::max);
    if lo.is_finite() && hi.is_finite() { (lo, hi) } else { (0.0, 0.0) }
}

/// Range of scatter x values (index + 1 where missing).
fn x_extent(c: &Chart) -> (f64, f64) {
    let xs: Vec<f64> = c.series.iter().flat_map(|s| s.xs.iter().flatten().copied()).collect();
    let lo = xs.iter().copied().fold(f64::INFINITY, f64::min);
    let hi = xs.iter().copied().fold(f64::NEG_INFINITY, f64::max);
    if lo.is_finite() && hi.is_finite() { (lo, hi) } else { (0.0, 1.0) }
}

fn bars(out: &mut Vec<GraphicItem>, c: &Chart, g: &Frame) {
    let stacked = c.stacked();
    let slots = if stacked { 1 } else { c.series.len().max(1) };
    let ov = if stacked { 0.0 } else { c.overlap };
    let ns = slots as f32;
    let band = g.band();
    let thick = band / (ns - (ns - 1.0) * ov + c.gap);
    let group = thick * (ns - (ns - 1.0) * ov);
    let mut pos = vec![0.0f64; g.n];
    let mut neg = vec![0.0f64; g.n];
    for (j, s) in c.series.iter().enumerate().filter(|(_, s)| has_values(s)) {
        let slot = if stacked { 0 } else { j };
        let slot = (if g.horiz { slots.saturating_sub(1 + slot) } else { slot }) as f32;
        for i in 0..g.n {
            let Some(v) = s.values.get(i).copied().flatten() else { continue };
            let (v0, v1) = if !stacked {
                (0.0, v)
            } else if v >= 0.0 {
                let Some(p) = pos.get_mut(i) else { continue };
                let b = *p;
                *p += v;
                (b, *p)
            } else {
                let Some(q) = neg.get_mut(i) else { continue };
                let b = *q;
                *q += v;
                (b, *q)
            };
            let a = g.band_start(i) + (band - group) / 2.0 + slot * thick * (1.0 - ov);
            let (p0, p1) = (g.val(v0), g.val(v1));
            let (p0, p1) = (p0.min(p1), p0.max(p1));
            let segs = if g.horiz { rect_segs(p0, a, p1, a + thick) } else { rect_segs(a, p0, a + thick, p1) };
            fill(out, segs, c.point_colour(j, i));
        }
    }
}

/// Whether a series has any point to draw.
fn has_values(s: &Series) -> bool {
    s.values.iter().any(Option::is_some)
}

/// Line series; stacked ones are drawn at the running total of the series up to them.
fn lines(out: &mut Vec<GraphicItem>, c: &Chart, g: &Frame) {
    let mut total = vec![0.0f64; g.n];
    for (j, s) in c.series.iter().enumerate().filter(|(_, s)| has_values(s)) {
        let pts: Vec<Option<(f32, f32)>> = (0..g.n)
            .map(|i| {
                let v = s.values.get(i).copied().flatten()?;
                let y = match total.get_mut(i) {
                    Some(t) if c.stacked() => {
                        *t += v;
                        *t
                    }
                    _ => v,
                };
                Some((g.cat_x(i), g.val(y)))
            })
            .collect();
        stroke(out, runs(pts.into_iter()), c.series_colour(j), s.line_w);
    }
}

fn areas(out: &mut Vec<GraphicItem>, c: &Chart, g: &Frame) {
    let mut base = vec![0.0f64; g.n];
    for (j, s) in c.series.iter().enumerate().filter(|(_, s)| has_values(s)) {
        let vals = (0..g.n).map(|i| s.values.get(i).copied().flatten().unwrap_or(0.0));
        let top: Vec<f64> = base.iter().zip(vals).map(|(b, v)| b + v).collect();
        let mut segs = Vec::with_capacity(2 * g.n + 1);
        for (i, t) in top.iter().enumerate() {
            let (x, y) = (g.cat_x(i), g.val(*t));
            segs.push(if i == 0 { PathSeg::Move(x, y) } else { PathSeg::Line(x, y) });
        }
        for (i, b) in base.iter().enumerate().rev() {
            segs.push(PathSeg::Line(g.cat_x(i), g.val(*b)));
        }
        segs.push(PathSeg::Close);
        fill(out, segs, c.series_colour(j));
        if c.stacked() {
            base = top;
        }
    }
}

/// Scatter series as lines, or as small markers when the series has no line.
fn draw_scatter(out: &mut Vec<GraphicItem>, c: &Chart, g: &Frame) {
    const MARKER: f32 = 6.0;
    for (j, s) in c.series.iter().enumerate() {
        let pts: Vec<Option<(f32, f32)>> =
            s.xs.iter()
                .zip(&s.values)
                .map(|(x, y)| match (x, y) {
                    (Some(x), Some(y)) => Some((g.scat_x(*x), g.val(*y))),
                    _ => None,
                })
                .collect();
        if s.line {
            stroke(out, runs(pts.iter().copied()), c.series_colour(j), s.line_w);
            continue;
        }
        for (x, y) in pts.iter().flatten() {
            let rect = [x - MARKER / 2.0, y - MARKER / 2.0, MARKER, MARKER];
            out.push(GraphicItem::Shape { rect, kind: ShapeKind::Ellipse, fill: Some(c.series_colour(j)), stroke: None, stroke_width: 0.0 });
        }
    }
}

fn pie(out: &mut Vec<GraphicItem>, c: &Chart, area: Rect) {
    let Some(s) = c.series.first() else { return };
    let vals: Vec<f64> = s.values.iter().map(|v| v.unwrap_or(0.0).max(0.0)).collect();
    let total: f64 = vals.iter().sum();
    let r = (area.w.min(area.h) / 2.0 - 2.0) as f64;
    if !(total > 0.0 && total.is_finite() && r >= 1.0) {
        return;
    }
    let (cx, cy) = ((area.x + area.w / 2.0) as f64, (area.y + area.h / 2.0) as f64);
    let inner = if c.kind == Kind::Doughnut { r * c.hole as f64 } else { 0.0 };
    let mut a = c.first_angle.to_radians();
    for (i, v) in vals.iter().enumerate() {
        if *v <= 0.0 {
            continue;
        }
        let b = a + v / total * TAU;
        let mut segs = Vec::new();
        if inner > 0.0 {
            segs.push(pt(polar(cx, cy, r, a)));
            arc(&mut segs, (cx, cy, r), a, b);
            segs.push(line_to(polar(cx, cy, inner, b)));
            arc(&mut segs, (cx, cy, inner), b, a);
        } else {
            segs.push(pt((cx, cy)));
            segs.push(line_to(polar(cx, cy, r, a)));
            arc(&mut segs, (cx, cy, r), a, b);
        }
        segs.push(PathSeg::Close);
        fill(out, segs, c.point_colour(0, i));
        a = b;
    }
}

/// A point at clockwise angle `t` (radians from 12 o'clock) on a circle of radius `r`.
fn polar(cx: f64, cy: f64, r: f64, t: f64) -> (f64, f64) {
    (cx + r * t.sin(), cy - r * t.cos())
}

fn pt(p: (f64, f64)) -> PathSeg {
    PathSeg::Move(p.0 as f32, p.1 as f32)
}

fn line_to(p: (f64, f64)) -> PathSeg {
    PathSeg::Line(p.0 as f32, p.1 as f32)
}

/// Cubic segments along a circle from angle `a` to `b` (the current point is at `a`). Each piece
/// spans at most a quarter turn.
fn arc(segs: &mut Vec<PathSeg>, (cx, cy, r): (f64, f64, f64), a: f64, b: f64) {
    let n = (((b - a).abs() / FRAC_PI_2).ceil() as usize).clamp(1, 16);
    let d = (b - a) / n as f64;
    let k = 4.0 / 3.0 * (d / 4.0).tan();
    for j in 0..n {
        let t0 = a + d * j as f64;
        let t1 = t0 + d;
        let p0 = polar(cx, cy, r, t0);
        let p1 = polar(cx, cy, r, t1);
        let c1 = (p0.0 + k * r * t0.cos(), p0.1 + k * r * t0.sin());
        let c2 = (p1.0 - k * r * t1.cos(), p1.1 - k * r * t1.sin());
        segs.push(PathSeg::Cubic(c1.0 as f32, c1.1 as f32, c2.0 as f32, c2.1 as f32, p1.0 as f32, p1.1 as f32));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wordcraft_doc::THEME_COLORS;

    const NS: &str =
        r#"xmlns:c="http://schemas.openxmlformats.org/drawingml/2006/chart" xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main""#;

    fn space(plot: &str) -> El {
        let x = format!(r#"<c:chartSpace {NS}><c:chart><c:plotArea>{plot}</c:plotArea></c:chart></c:chartSpace>"#);
        crate::xml::parse(x.as_bytes()).unwrap()
    }

    fn theme() -> Vec<Rgb> {
        THEME_COLORS.to_vec()
    }

    fn ser(i: usize, name: &str, color: Option<&str>, cats: &[&str], vals: &[&str]) -> String {
        let fill = color.map_or(String::new(), |c| format!(r#"<c:spPr><a:solidFill><a:srgbClr val="{c}"/></a:solidFill></c:spPr>"#));
        let cat_pts: String = cats.iter().enumerate().map(|(k, c)| format!(r#"<c:pt idx="{k}"><c:v>{c}</c:v></c:pt>"#)).collect();
        let val_pts: String = vals.iter().enumerate().map(|(k, v)| format!(r#"<c:pt idx="{k}"><c:v>{v}</c:v></c:pt>"#)).collect();
        format!(
            r#"<c:ser><c:idx val="{i}"/><c:order val="{i}"/><c:tx><c:strRef><c:strCache><c:ptCount val="1"/><c:pt idx="0"><c:v>{name}</c:v></c:pt></c:strCache></c:strRef></c:tx>{fill}<c:cat><c:strRef><c:strCache><c:ptCount val="{nc}"/>{cat_pts}</c:strCache></c:strRef></c:cat><c:val><c:numRef><c:numCache><c:formatCode>General</c:formatCode><c:ptCount val="{nv}"/>{val_pts}</c:numCache></c:numRef></c:val></c:ser>"#,
            nc = cats.len(),
            nv = vals.len()
        )
    }

    fn filled(items: &[GraphicItem]) -> Vec<(Rgb, f32, f32)> {
        items
            .iter()
            .filter_map(|it| match it {
                GraphicItem::Path { segs, fill: Some(c), .. } => {
                    let ys: Vec<f32> = segs
                        .iter()
                        .filter_map(|s| match s {
                            PathSeg::Move(_, y) | PathSeg::Line(_, y) => Some(*y),
                            _ => None,
                        })
                        .collect();
                    let lo = ys.iter().copied().fold(f32::INFINITY, f32::min);
                    let hi = ys.iter().copied().fold(f32::NEG_INFINITY, f32::max);
                    Some((*c, lo, hi))
                }
                _ => None,
            })
            .collect()
    }

    #[test]
    fn clustered_columns_draw_one_bar_per_point() {
        let plot = format!(
            r#"<c:barChart><c:barDir val="col"/><c:grouping val="clustered"/>{}{}<c:axId val="1"/><c:axId val="2"/></c:barChart><c:catAx><c:axId val="1"/></c:catAx><c:valAx><c:axId val="2"/><c:majorGridlines/></c:valAx>"#,
            ser(0, "North", Some("FF0000"), &["Q1", "Q2", "Q3"], &["4", "8", "2"]),
            ser(1, "South", Some("00FF00"), &["Q1", "Q2", "Q3"], &["2", "6", "4"]),
        );
        let items = chart_items(&space(&plot), &theme(), 300.0, 200.0);
        let bars = filled(&items);
        assert_eq!(bars.len(), 6);
        let heights: Vec<f32> = bars.iter().map(|(_, lo, hi)| hi - lo).collect();
        // Scale 0..8: North Q2 is the full plot height, North Q1 half of it.
        assert!((heights[1] / heights[0] - 2.0).abs() < 1e-3, "{heights:?}");
        // South (second series) Q1 is 2 of 8, Q2 is 6 of 8.
        assert!((heights[3] / heights[1] - 0.25).abs() < 1e-3, "{heights:?}");
        assert!((heights[4] / heights[1] - 0.75).abs() < 1e-3, "{heights:?}");
        assert_eq!(bars[0].0, Rgb(0xFF, 0, 0));
        assert_eq!(bars[3].0, Rgb(0, 0xFF, 0));
        assert!(items.iter().any(|it| matches!(it, GraphicItem::Text { text, .. } if text == "Q2")));
    }

    #[test]
    fn pie_draws_one_slice_per_point() {
        let plot =
            format!(r#"<c:pieChart><c:varyColors val="1"/>{}</c:pieChart>"#, ser(0, "Share", None, &["A", "B", "C", "D"], &["50", "25", "15", "10"]));
        let items = chart_items(&space(&plot), &theme(), 200.0, 150.0);
        let slices = filled(&items);
        assert_eq!(slices.len(), 4);
        assert_eq!(slices[0].0, theme()[4], "first slice is accent1");
    }

    #[test]
    fn doughnut_slices_are_annuli() {
        let plot = format!(
            r#"<c:doughnutChart><c:varyColors val="1"/>{}<c:holeSize val="50"/></c:doughnutChart>"#,
            ser(0, "S", None, &["A", "B"], &["1", "3"])
        );
        let items = chart_items(&space(&plot), &theme(), 200.0, 150.0);
        assert_eq!(items.iter().filter(|it| matches!(it, GraphicItem::Path { fill: Some(_), .. })).count(), 2);
    }

    #[test]
    fn nice_scale_rounds_to_round_steps() {
        assert_eq!(nice_scale(0.0, 22.3, true, None, None), Scale { lo: 0.0, hi: 25.0, step: 5.0 });
        assert_eq!(nice_scale(0.0, 100.0, true, None, None), Scale { lo: 0.0, hi: 120.0, step: 20.0 });
        assert_eq!(nice_scale(0.0, 22.3, true, Some(0.0), Some(30.0)).hi, 30.0);
        let flat = nice_scale(0.0, 0.0, true, None, None);
        assert!(flat.lo == 0.0 && (flat.hi - 1.2).abs() < 1e-9 && flat.step == 0.2, "{flat:?}");
    }

    #[test]
    fn number_formats() {
        assert_eq!(format_value(1234567.0, "#,##0", 1.0), "1,234,567");
        assert_eq!(format_value(-1500.0, "#,##0", 500.0), "-1,500");
        assert_eq!(format_value(0.25, "0%", 0.05), "25%");
        assert_eq!(format_value(0.5, "General", 0.25), "0.50");
        assert_eq!(format_value(10.0, "\"€\"0.00", 5.0), "€10.00");
    }

    #[test]
    fn hostile_values_never_panic() {
        let bodies = [
            format!(
                r#"<c:barChart><c:barDir val="col"/><c:grouping val="stacked"/><c:gapWidth val="-9999999999999999999999"/><c:overlap val="NaN"/>{}<c:axId val="1"/></c:barChart><c:valAx><c:axId val="1"/><c:scaling><c:max val="1e400"/><c:min val="-1e308"/></c:scaling></c:valAx>"#,
                r#"<c:ser><c:tx><c:v>x</c:v></c:tx><c:cat><c:strRef><c:strCache><c:ptCount val="99999999999"/><c:pt idx="18446744073709551615"><c:v>a</c:v></c:pt><c:pt idx="-3"><c:v>b</c:v></c:pt></c:strCache></c:strRef></c:cat><c:val><c:numRef><c:numCache><c:ptCount val="3"/><c:pt idx="0"><c:v>1e400</c:v></c:pt><c:pt idx="1"><c:v>NaN</c:v></c:pt><c:pt idx="2"><c:v>-1e308</c:v></c:pt><c:pt idx="99999"><c:v>5</c:v></c:pt></c:numCache></c:numRef></c:val></c:ser><c:ser><c:val><c:numRef><c:numCache><c:pt idx="0"><c:v>1e308</c:v></c:pt><c:pt idx="0"><c:v>1e308</c:v></c:pt></c:numCache></c:numRef></c:val></c:ser>"#
            ),
            r#"<c:scatterChart><c:ser><c:xVal><c:numLit><c:ptCount val="2"/><c:pt idx="0"><c:v>-1e308</c:v></c:pt><c:pt idx="1"><c:v>1e308</c:v></c:pt></c:numLit></c:xVal><c:yVal><c:numLit><c:ptCount val="2"/><c:pt idx="0"><c:v>1</c:v></c:pt><c:pt idx="1"><c:v>2</c:v></c:pt></c:numLit></c:yVal></c:ser></c:scatterChart>"#.to_string(),
            r#"<c:pieChart><c:ser><c:val><c:numLit><c:ptCount val="2"/><c:pt idx="0"><c:v>-4</c:v></c:pt><c:pt idx="1"><c:v>1e308</c:v></c:pt></c:numLit></c:val><c:dPt><c:idx val="9999999999999"/></c:dPt></c:ser><c:firstSliceAng val="-9"/></c:pieChart>"#.to_string(),
        ];
        for body in &bodies {
            let s = space(body);
            for (w, h) in [(0.0, 10.0), (f32::NAN, 10.0), (-1.0, 1.0), (1e30, 1e30), (50.0, 40.0)] {
                let _ = chart_items(&s, &theme(), w, h);
            }
        }
        assert!(chart_items(&space(&bodies[0]), &theme(), f32::NAN, 10.0).is_empty());
        assert!(chart_items(&El::default(), &theme(), 100.0, 100.0).is_empty());
    }

    #[test]
    fn unsupported_chart_types_draw_nothing() {
        let plot = r#"<c:radarChart><c:ser><c:val><c:numLit><c:ptCount val="1"/><c:pt idx="0"><c:v>1</c:v></c:pt></c:numLit></c:val></c:ser></c:radarChart>"#;
        assert!(chart_items(&space(plot), &theme(), 100.0, 100.0).is_empty());
    }

    /// Chart items for a chart with a legend at `pos` (`r`, `b`…).
    fn with_legend(plot: &str, pos: &str, w: f32, h: f32) -> Vec<GraphicItem> {
        let x = format!(
            r#"<c:chartSpace {NS}><c:chart><c:plotArea>{plot}</c:plotArea><c:legend><c:legendPos val="{pos}"/></c:legend></c:chart></c:chartSpace>"#
        );
        chart_items(&crate::xml::parse(x.as_bytes()).unwrap(), &theme(), w, h)
    }

    #[test]
    fn series_are_capped_to_the_cell_budget() {
        // 256 series each declaring 10,000 points: only the 10 that fit the cell budget are kept.
        let sers: String = (0..256)
            .map(|i| format!(r#"<c:ser><c:idx val="{i}"/><c:val><c:numRef><c:numCache><c:ptCount val="10000"/><c:pt idx="0"><c:v>1</c:v></c:pt></c:numCache></c:numRef></c:val></c:ser>"#))
            .collect();
        let plot = format!(r#"<c:lineChart><c:grouping val="standard"/>{sers}<c:axId val="1"/></c:lineChart>"#);
        let items = chart_items(&space(&plot), &theme(), 300.0, 200.0);
        let strokes = items.iter().filter(|it| matches!(it, GraphicItem::Path { fill: None, stroke: Some(_), .. })).count();
        assert_eq!(strokes, MAX_CELLS / 10_000);
    }

    #[test]
    fn series_without_values_draw_nothing() {
        let plot = format!(
            r#"<c:areaChart><c:grouping val="standard"/>{}{}<c:axId val="1"/></c:areaChart>"#,
            ser(0, "A", Some("FF0000"), &["Q1", "Q2"], &["4", "8"]),
            ser(1, "Empty", Some("00FF00"), &["Q1", "Q2"], &[]),
        );
        let fills = filled(&chart_items(&space(&plot), &theme(), 300.0, 200.0));
        assert_eq!(fills.len(), 1);
        assert_eq!(fills[0].0, Rgb(0xFF, 0, 0));
    }

    #[test]
    fn value_scale_starts_bars_at_zero_for_negative_data() {
        // All-negative data pads below only: the axis ends at zero, as positive data's starts there.
        assert_eq!(nice_scale(-10.0, -2.0, true, None, None), Scale { lo: -12.0, hi: 0.0, step: 2.0 });
        assert_eq!(nice_scale(-1050.0, -1000.0, false, None, None).hi, -1000.0);
    }

    #[test]
    fn narrow_line_scales_leave_zero_out_but_bars_keep_it() {
        assert!(narrow(1000.0, 1050.0) && narrow(-1050.0, -1000.0));
        assert!(!narrow(10.0, 100.0) && !narrow(0.0, 1.0));
        assert_eq!(nice_scale(1000.0, 1050.0, false, None, None).lo, 1000.0);
        assert_eq!(nice_scale(1000.0, 1050.0, true, None, None).lo, 0.0);
    }

    #[test]
    fn horizontal_bar_labels_wrap_in_a_column_of_forty_percent() {
        let long = "A category label that is much too long for its column";
        let plot = format!(
            r#"<c:barChart><c:barDir val="bar"/><c:grouping val="clustered"/>{}<c:axId val="1"/><c:axId val="2"/></c:barChart><c:catAx><c:axId val="1"/></c:catAx><c:valAx><c:axId val="2"/></c:valAx>"#,
            ser(0, "S", Some("FF0000"), &[long, "Short"], &["4", "8"]),
        );
        let items = chart_items(&space(&plot), &theme(), 300.0, 200.0);
        let labels: Vec<(&str, f32)> = items
            .iter()
            .filter_map(|it| match it {
                GraphicItem::Text { rect, text, align: TextAlign::Right, .. } => Some((text.as_str(), rect[2])),
                _ => None,
            })
            .collect();
        assert!(labels.iter().all(|(_, w)| *w <= 0.4 * 300.0), "{labels:?}");
        assert!(labels.iter().filter(|(t, _)| long.contains(t)).count() > 1, "wrapped: {labels:?}");
    }

    #[test]
    fn stacked_lines_add_up_like_areas() {
        let stacked = format!(
            r#"<c:lineChart><c:grouping val="stacked"/>{}{}<c:axId val="1"/></c:lineChart>"#,
            ser(0, "A", Some("FF0000"), &["Q1", "Q2"], &["0", "10"]),
            ser(1, "B", Some("00FF00"), &["Q1", "Q2"], &["0", "5"]),
        );
        let single = format!(
            r#"<c:lineChart><c:grouping val="standard"/>{}<c:axId val="1"/></c:lineChart>"#,
            ser(0, "T", Some("0000FF"), &["Q1", "Q2"], &["0", "15"])
        );
        // Where the last point of the line drawn in `colour` is.
        let end_y = |items: &[GraphicItem], colour: Rgb| {
            items.iter().find_map(|it| match it {
                GraphicItem::Path { segs, stroke: Some(c), .. } if *c == colour => match segs.last() {
                    Some(PathSeg::Line(_, y)) => Some(*y),
                    _ => None,
                },
                _ => None,
            })
        };
        let top = end_y(&chart_items(&space(&stacked), &theme(), 300.0, 200.0), Rgb(0, 0xFF, 0)).expect("stacked line");
        let total = end_y(&chart_items(&space(&single), &theme(), 300.0, 200.0), Rgb(0, 0, 0xFF)).expect("single line");
        assert!((top - total).abs() < 1e-3, "stacked {top} vs {total}");
    }

    #[test]
    fn category_numbers_show_in_their_format() {
        // Excel serial days: 45292 is 1 January 2024, 45323 is 1 February 2024.
        let cat = |code: &str| -> Vec<String> {
            let x = format!(
                r#"<c:cat {NS}><c:numRef><c:numCache><c:formatCode>{code}</c:formatCode><c:ptCount val="2"/><c:pt idx="0"><c:v>45292</c:v></c:pt><c:pt idx="1"><c:v>45323</c:v></c:pt></c:numCache></c:numRef></c:cat>"#
            );
            labels(Some(&crate::xml::parse(x.as_bytes()).unwrap()), 10, false)
        };
        assert_eq!(cat("m/d/yyyy"), ["1/1/2024", "2/1/2024"]);
        assert_eq!(cat("d/m/yyyy"), ["1/1/2024", "1/2/2024"]);
        assert_eq!(cat("mmm-yy"), ["Jan-24", "Feb-24"]);
        assert_eq!(cat("yyyy"), ["2024", "2024"]);
        assert_eq!(cat("0.00"), ["45292.00", "45323.00"]);
        let ymd = |v: f64| excel_stamp(v, false).map(|t| (t.y, t.m, t.d));
        assert_eq!(ymd(61.0), Some((1900, 3, 1)));
        assert_eq!(ymd(45292.0), Some((2024, 1, 1)));
    }

    #[test]
    fn number_formats_keep_literal_text_and_currency() {
        assert_eq!(format_value(1234.5, "#,##0.00 [$€-407]", 0.25), "1,234.50 €");
        assert_eq!(format_value(5.0, "[Red]0", 1.0), "5");
        assert_eq!(format_value(3.0, "\"Items: \"0", 1.0), "Items: 3");
        assert_eq!(format_value(3.0, "0\" units\"", 1.0), "3 units");
    }

    #[test]
    fn side_legend_keeps_only_the_entries_that_fit() {
        let sers: String = (0..10).map(|i| ser(i, &format!("S{i}"), Some("FF0000"), &["Q1"], &["1"])).collect();
        let plot = format!(
            r#"<c:barChart><c:barDir val="col"/><c:grouping val="clustered"/>{sers}<c:axId val="1"/><c:axId val="2"/></c:barChart><c:catAx><c:axId val="1"/></c:catAx><c:valAx><c:axId val="2"/></c:valAx>"#
        );
        let items = with_legend(&plot, "r", 300.0, 60.0);
        let swatches = items.iter().filter(|it| matches!(it, GraphicItem::Shape { rect, .. } if rect[2] == 7.0)).count();
        assert!(swatches > 0 && swatches < 10, "{swatches} swatches");
        let bottom =
            items.iter().filter_map(|it| if let GraphicItem::Text { rect, .. } = it { Some(rect[1] + rect[3]) } else { None }).fold(0.0, f32::max);
        assert!(bottom <= 60.0, "legend text passes the chart's bottom: {bottom}");
    }

    /// The swatches of a legend: their (x, y).
    fn swatches(items: &[GraphicItem]) -> Vec<(f32, f32)> {
        items
            .iter()
            .filter_map(|it| if let GraphicItem::Shape { rect, .. } = it { (rect[2] == SWATCH).then_some((rect[0], rect[1])) } else { None })
            .collect()
    }

    /// A column chart of one category with a series per name.
    fn columns(names: &[&str]) -> String {
        let sers: String = names.iter().enumerate().map(|(i, n)| ser(i, n, Some("FF0000"), &["Q1"], &["1"])).collect();
        format!(
            r#"<c:barChart><c:barDir val="col"/><c:grouping val="clustered"/>{sers}<c:axId val="1"/><c:axId val="2"/></c:barChart><c:catAx><c:axId val="1"/></c:catAx><c:valAx><c:axId val="2"/></c:valAx>"#
        )
    }

    #[test]
    fn bottom_legend_wraps_into_rows_inside_the_chart() {
        let names: Vec<String> = (0..12).map(|i| format!("Region number {i}")).collect();
        let names: Vec<&str> = names.iter().map(String::as_str).collect();
        let items = with_legend(&columns(&names), "b", 200.0, 200.0);
        let sw = swatches(&items);
        let rows: std::collections::BTreeSet<i32> = sw.iter().map(|(_, y)| *y as i32).collect();
        assert!(rows.len() > 1 && rows.len() <= MAX_LEGEND_ROWS, "rows {rows:?}");
        assert!(sw.iter().all(|(x, _)| *x >= 0.0 && *x <= 200.0), "{sw:?}");
        for it in &items {
            if let GraphicItem::Text { rect, text, .. } = it {
                assert!(rect[0] >= 0.0 && rect[0] + text_w(text, FONT) <= 200.0 + 1e-3, "{text:?} at {rect:?}");
            }
        }
    }

    #[test]
    fn long_legend_names_are_cut_to_a_share_of_the_chart() {
        let long = "A very long series name that would push the legend past the left edge of the chart";
        let items = with_legend(&columns(&[long, "Short"]), "r", 200.0, 150.0);
        assert!(swatches(&items).iter().all(|(x, _)| *x >= 0.0), "{:?}", swatches(&items));
        let cut = items.iter().find_map(|it| match it {
            GraphicItem::Text { text, rect, .. } if text.ends_with('…') => Some((text.clone(), rect[2])),
            _ => None,
        });
        let (text, w) = cut.expect("the long name is cut");
        assert!(text_w(&text, FONT) <= 200.0 * LEGEND_SHARE && w <= 200.0 * LEGEND_SHARE + 2.0, "{text:?} {w}");
    }

    #[test]
    fn scatter_dates_show_as_dates() {
        let pts = |vals: &[&str]| -> String { vals.iter().enumerate().map(|(k, v)| format!(r#"<c:pt idx="{k}"><c:v>{v}</c:v></c:pt>"#)).collect() };
        let plot = format!(
            r#"<c:scatterChart><c:ser><c:idx val="0"/><c:xVal><c:numRef><c:numCache><c:formatCode>m/d/yyyy</c:formatCode><c:ptCount val="3"/>{}</c:numCache></c:numRef></c:xVal><c:yVal><c:numRef><c:numCache><c:formatCode>General</c:formatCode><c:ptCount val="3"/>{}</c:numCache></c:numRef></c:yVal></c:ser><c:axId val="1"/><c:axId val="2"/></c:scatterChart><c:valAx><c:axId val="1"/><c:axPos val="b"/></c:valAx><c:valAx><c:axId val="2"/><c:axPos val="l"/></c:valAx>"#,
            pts(&["45292", "45323", "45352"]),
            pts(&["1", "2", "3"])
        );
        let items = chart_items(&space(&plot), &theme(), 300.0, 200.0);
        let texts: Vec<&str> =
            items.iter().filter_map(|it| if let GraphicItem::Text { text, .. } = it { Some(text.as_str()) } else { None }).collect();
        assert!(texts.iter().any(|t| t.ends_with("/2024")), "{texts:?}");
        assert!(!texts.iter().any(|t| t.starts_with("45")), "serials shown: {texts:?}");
    }

    #[test]
    fn date1904_counts_from_1904() {
        assert_eq!(category_text(0.0, "m/d/yyyy", 1.0, true), "1/1/1904");
        assert_eq!(category_text(1462.0, "m/d/yyyy", 1.0, false), "1/1/1904");
        let x = format!(
            r#"<c:chartSpace {NS}><c:date1904 val="1"/><c:chart><c:plotArea><c:lineChart><c:ser><c:cat><c:numRef><c:numCache><c:formatCode>yyyy-mm-dd</c:formatCode><c:ptCount val="1"/><c:pt idx="0"><c:v>366</c:v></c:pt></c:numCache></c:numRef></c:cat><c:val><c:numLit><c:ptCount val="1"/><c:pt idx="0"><c:v>1</c:v></c:pt></c:numLit></c:val></c:ser></c:lineChart><c:catAx/><c:valAx/></c:plotArea></c:chart></c:chartSpace>"#
        );
        let chart = Chart::parse(&crate::xml::parse(x.as_bytes()).unwrap(), &theme()).expect("chart");
        assert_eq!(chart.cats, ["1905-01-01"]);
    }

    #[test]
    fn date_codes_show_times_weekdays_and_early_serials() {
        let t = |v: f64, code: &str| category_text(v, code, 1.0, false);
        // 45292 is Monday 1 January 2024; 13:05:09 into it.
        let at = 45292.0 + (13.0 * 3600.0 + 5.0 * 60.0 + 9.0) / 86_400.0;
        assert_eq!(t(at, "hh:mm:ss"), "13:05:09");
        assert_eq!(t(at, "h:mm AM/PM"), "1:05 PM");
        assert_eq!(t(at, "m:ss"), "5:09");
        assert_eq!(t(45292.0, "h:mm a/p"), "12:00 A");
        assert_eq!(t(at, "mm/dd"), "01/01");
        assert_eq!(t(45292.0, "dddd, mmmm d"), "Monday, January 1");
        assert_eq!(t(45292.0, "ddd d mmm"), "Mon 1 Jan");
        // Serials before March 1900, Excel's phantom 29 February included.
        assert_eq!(t(0.0, "m/d/yyyy"), "1/0/1900");
        assert_eq!(t(1.0, "m/d/yyyy"), "1/1/1900");
        assert_eq!(t(59.0, "m/d/yyyy"), "2/28/1900");
        assert_eq!(t(60.0, "m/d/yyyy"), "2/29/1900");
        assert_eq!(t(61.0, "m/d/yyyy dddd"), "3/1/1900 Thursday");
        // Outside the calendar: a number.
        assert_eq!(t(-1.0, "m/d/yyyy"), "-1");
    }

    #[test]
    fn negative_sections_and_scientific_numbers() {
        assert_eq!(format_value(-5.0, "#,##0;(#,##0)", 1.0), "(5)");
        assert_eq!(format_value(5.0, "#,##0;(#,##0)", 1.0), "5");
        assert_eq!(format_value(-1500.0, "#,##0_);(#,##0)", 500.0), "(1,500)");
        assert_eq!(format_value(-2.0, "0\";\"0;-0", 1.0), "-2");
        assert_eq!(format_value(12345.0, "0.00E+00", 1.0), "1.23E+04");
        assert_eq!(format_value(0.00012, "0.0E+0", 0.0001), "1.2E-4");
        assert_eq!(format_value(0.0, "0.00E+00", 1.0), "0.00E+00");
        assert_eq!(format_value(-12345.0, "0.0E-00", 1.0), "-1.2E04");
    }
}
