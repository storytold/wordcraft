//! Charts (ECMA-376 Part 1 §21.2): the cached data of a `c:chartSpace` drawn as items in points.
//!
//! Bar and column (clustered, stacked or 100% stacked), line, area, pie, doughnut and scatter
//! charts are drawn from the first supported chart type of the plot area, with their values as
//! data labels where `c:dLbls` shows them (`c:showVal`). Radar charts (standard, with markers or
//! filled) draw as spokes, concentric grid polygons and one polygon per series. A chart of only other types (stock, bubble…)
//! draws a placeholder: its frame, its title and the type it is. Text widths are estimated ([`CHAR_W`] em a character), so label places are approximate.

use std::f64::consts::{FRAC_PI_2, TAU};

use wordcraft_doc::graphic::{GraphicItem, PathSeg, TextAlign};
use wordcraft_doc::para::ShapeKind;
use wordcraft_doc::props::Rgb;

use super::drawing_color::{fill_colour, slot};
use crate::units::{int, measure};
use crate::xml::El;

mod canvas;
mod format;
mod model;
mod parse;
mod plot;
mod polar;

#[cfg(test)]
use canvas::SWATCH;
pub(super) use canvas::text_w;
use canvas::{Rect, clip_text, legend, text};
#[cfg(test)]
use format::{civil, excel_stamp, format_value};
use model::{Axis, Grouping, Kind, Pos, Series};
#[cfg(test)]
use parse::{axis, category_text, labels, numbers};
use parse::{read_series, text_size, title_of};
#[cfg(test)]
use plot::{Scale, narrow, nice_scale};
use plot::{backdrop, cartesian};
use polar::{pie, radar};

/// Most series read from one chart.
const MAX_SERIES: usize = 256;
/// Most points read from one series.
const MAX_POINTS: usize = 10_000;
/// Most points read from one chart, all series together.
const MAX_CELLS: usize = 100_000;
/// Longest series name or category label kept.
const MAX_LABEL: usize = 200;
/// Most categories (spokes) of a radar chart.
const MAX_SPOKES: usize = 100;
/// Most series of a radar chart.
const MAX_RADAR_SERIES: usize = 50;
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

/// Items for a chart part, sized `w` x `h` points: a placeholder when only unsupported chart
/// types are in it, empty when there's nothing to draw.
pub(crate) fn chart_items(space: &El, theme: &[Rgb], w: f32, h: f32) -> Vec<GraphicItem> {
    let mut out = Vec::new();
    if !(w.is_finite() && h.is_finite() && w > 0.0 && h > 0.0) {
        return out;
    }
    let Some(chart) = Chart::parse(space, theme) else {
        placeholder(&mut out, space, theme, w, h);
        return out;
    };
    backdrop(&mut out, space, theme, w, h);
    let mut area = Rect { x: PAD, y: PAD, w: (w - 2.0 * PAD).max(1.0), h: (h - 2.0 * PAD).max(1.0) };
    if let Some(t) = &chart.title {
        area = title_block(&mut out, t, area);
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
        Kind::Radar => radar(&mut out, &chart, area),
        _ => cartesian(&mut out, &chart, area),
    }
    out
}

/// The title at the top of `area`; returns the room left under it.
fn title_block(out: &mut Vec<GraphicItem>, t: &str, area: Rect) -> Rect {
    let lh = TITLE_SIZE * LINE;
    text(out, Rect { x: area.x, y: area.y, w: area.w, h: lh }, t, TITLE_SIZE, TextAlign::Center, true);
    Rect { y: area.y + lh + PAD, h: (area.h - lh - PAD).max(1.0), ..area }
}

/// A chart whose plot area holds only chart types WordCraft doesn't draw (stock, bubble…): its
/// frame with a light border, its title and a line saying which type it is, so it isn't blank
/// space. Nothing when the plot area has no chart type at all, or a supported one with nothing
/// to draw.
fn placeholder(out: &mut Vec<GraphicItem>, space: &El, theme: &[Rgb], w: f32, h: f32) {
    let Some(chart) = space.child("c:chart") else { return };
    let Some(plot) = chart.child("c:plotArea") else { return };
    if plot.els().any(|e| Kind::of(&e.name).is_some()) {
        return;
    }
    let Some(ty) = plot.els().find(|e| e.name.ends_with("Chart")) else { return };
    backdrop(out, space, theme, w, h);
    out.push(GraphicItem::Shape { rect: [0.0, 0.0, w, h], kind: ShapeKind::Rectangle, fill: None, stroke: Some(AXIS), stroke_width: 0.75 });
    let mut area = Rect { x: PAD, y: PAD, w: (w - 2.0 * PAD).max(1.0), h: (h - 2.0 * PAD).max(1.0) };
    if let Some(t) = title_of(chart, &read_series(ty, false, theme)) {
        area = title_block(out, &t, area);
    }
    let size = text_size(space);
    let lh = size * LINE;
    let msg = format!("{} (not supported yet)", unsupported_name(&ty.name));
    let line = Rect { x: area.x, y: area.y + ((area.h - lh) / 2.0).max(0.0), w: area.w, h: lh };
    text(out, line, &clip_text(&msg, area.w, size), size, TextAlign::Center, false);
}

/// What a placeholder calls a chart type element WordCraft doesn't draw.
fn unsupported_name(name: &str) -> &'static str {
    match name {
        "c:stockChart" => "Stock chart",
        "c:bubbleChart" => "Bubble chart",
        "c:surfaceChart" | "c:surface3DChart" => "Surface chart",
        "c:ofPieChart" => "Pie of pie chart",
        "c:bar3DChart" => "3-D bar chart",
        "c:line3DChart" => "3-D line chart",
        "c:area3DChart" => "3-D area chart",
        "c:pie3DChart" => "3-D pie chart",
        _ => "Chart",
    }
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
    /// Radar: series polygons are filled (`c:radarStyle` = `filled`).
    radar_fill: bool,
    /// Radar: points carry markers (`c:radarStyle` = `marker`).
    radar_marks: bool,
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

#[cfg(test)]
mod tests;
