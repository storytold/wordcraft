use super::canvas::{Rect, fill, rect_segs, runs, seg, stroke, text, text_at, text_w, widest, wrap};
use super::format::format_value;
use super::model::{DataLabel, Grouping, Kind, Series};
use super::parse::category_text;
use super::polar::{draw_data_labels, label_of};
use super::*;

pub(super) fn label(s: &str) -> String {
    s.chars().take(MAX_LABEL).collect()
}

/// Chart area: the chart space's fill and border (white and borderless by default).
pub(super) fn backdrop(out: &mut Vec<GraphicItem>, space: &El, theme: &[Rgb], w: f32, h: f32) {
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
pub(super) struct Scale {
    pub(super) lo: f64,
    pub(super) hi: f64,
    pub(super) step: f64,
}

impl Scale {
    pub(super) fn ticks(&self) -> Vec<f64> {
        let n = (((self.hi - self.lo) / self.step + 1e-9).floor().max(0.0) as usize).min(MAX_TICKS);
        (0..=n).map(|k| self.lo + k as f64 * self.step).filter(|t| *t <= self.hi + 1e-9).collect()
    }
}

/// A scale for data `dlo..dhi` (zero included when `zero`), as Excel and Word pick it: an end
/// away from zero sits 5% of the data span beyond the data (an end at zero stays there), rounded
/// out to a step of 1, 2, 2.5, 5 or 10 times a power of ten (about seven steps). `min`/`max`
/// from the file replace the ends.
pub(super) fn nice_scale(dlo: f64, dhi: f64, zero: bool, min: Option<f64>, max: Option<f64>) -> Scale {
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
pub(super) struct Axes {
    pub(super) vs: Scale,
    vticks: Vec<(f64, String)>,
    xs: Option<Scale>,
    xticks: Vec<(f64, String)>,
}

impl Axes {
    pub(super) fn of(c: &Chart) -> Axes {
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
pub(super) fn cartesian(out: &mut Vec<GraphicItem>, c: &Chart, area: Rect) {
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
    let mut labels = Vec::new();
    match c.kind {
        Kind::Bar => bars(out, c, &g, &mut labels),
        Kind::Line => lines(out, c, &g, &mut labels),
        Kind::Area => areas(out, c, &g, &mut labels),
        Kind::Scatter => draw_scatter(out, c, &g, &mut labels),
        Kind::Pie | Kind::Doughnut | Kind::Radar => {}
    }
    draw_axes(out, c, &g);
    draw_data_labels(out, &labels, c.text_size, g.horiz && c.kind == Kind::Bar);
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
pub(super) fn narrow(lo: f64, hi: f64) -> bool {
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

fn bars(out: &mut Vec<GraphicItem>, c: &Chart, g: &Frame, labels: &mut Vec<DataLabel>) {
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
            if let Some(text) = label_of(s, i) {
                let mid = a + thick / 2.0;
                let (x, y) = match (g.horiz, stacked) {
                    (true, true) => ((p0 + p1) / 2.0, mid),
                    (false, true) => (mid, (p0 + p1) / 2.0),
                    // Beyond the bar's end: right of a positive bar, above a positive column.
                    (true, false) => (if v >= 0.0 { p1 } else { p0 }, mid),
                    (false, false) => (mid, if v >= 0.0 { p0 } else { p1 }),
                };
                labels.push(DataLabel { x, y, text, inside: stacked });
            }
        }
    }
}

/// Whether a series has any point to draw.
pub(super) fn has_values(s: &Series) -> bool {
    s.values.iter().any(Option::is_some)
}

/// Line series; stacked ones are drawn at the running total of the series up to them.
fn lines(out: &mut Vec<GraphicItem>, c: &Chart, g: &Frame, labels: &mut Vec<DataLabel>) {
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
        for (i, p) in pts.iter().enumerate() {
            if let (Some((x, y)), Some(text)) = (p, label_of(s, i)) {
                labels.push(DataLabel { x: *x, y: *y, text, inside: false });
            }
        }
        stroke(out, runs(pts.into_iter()), c.series_colour(j), s.line_w);
    }
}

fn areas(out: &mut Vec<GraphicItem>, c: &Chart, g: &Frame, labels: &mut Vec<DataLabel>) {
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
        for (i, (t, b)) in top.iter().zip(&base).enumerate() {
            if let Some(text) = label_of(s, i) {
                labels.push(DataLabel { x: g.cat_x(i), y: g.val((t + b) / 2.0), text, inside: true });
            }
        }
        if c.stacked() {
            base = top;
        }
    }
}

/// Scatter series as lines, or as small markers when the series has no line.
fn draw_scatter(out: &mut Vec<GraphicItem>, c: &Chart, g: &Frame, labels: &mut Vec<DataLabel>) {
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
        for (i, p) in pts.iter().enumerate() {
            if let (Some((x, y)), Some(text)) = (p, label_of(s, i)) {
                labels.push(DataLabel { x: *x, y: *y - MARKER / 2.0, text, inside: false });
            }
        }
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
