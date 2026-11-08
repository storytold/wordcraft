use super::canvas::{Rect, fill, rect_segs, runs, seg, stroke, text, text_at, text_w, widest, wrap};
use super::format::format_value;
use super::model::{DataLabel, Grouping, Kind, Series};
use super::parse::category_text;
use super::polar::{draw_data_labels, label_of};
use super::{AXIS, Chart, El, GAP, GRID, GraphicItem, LINE, MAX_LABEL, MAX_TICKS, PAD, PathSeg, Rgb, ShapeKind, TextAlign, fill_colour, measure};

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
    /// The value scale of `c` and, for scatter and bubble charts, its x scale, with their ticks and
    /// tick texts.
    pub(super) fn of(c: &Chart) -> Axes {
        Axes::fitting(c, &[], 0.0, 0.0)
    }

    /// [`Axes::of`], with automatic axis ends widened so that each of `bubbles` fits whole in a
    /// plot `w` x `h` points, as Word widens them (measured: x 1..3 with sizes 1, 4 and 16 at
    /// 432 x 252 gets 0..3.5, and data 1..10 with the largest bubble at the top gets y -2..12).
    fn fitting(c: &Chart, bubbles: &[Bubble], w: f32, h: f32) -> Axes {
        let (dlo, dhi) = extent(c);
        let (vmin, vmax) = if c.grouping == Grouping::Percent { (Some(0.0), Some(1.0)) } else { (c.val.min, c.val.max) };
        // Lines, scatters and bubbles don't need zero for a narrow range (1000..1050 stays on its
        // own scale). Word's bubble charts take zero on both axes, and for a single value too
        // (x = 3 gets 0..3.5).
        let bubble = c.kind == Kind::Bubble;
        let zero_for = |lo: f64, hi: f64| (bubble && lo == hi) || !narrow(lo, hi);
        let zero = !(c.kind == Kind::Line || c.kind.xy()) || zero_for(dlo, dhi);
        let radius = |b: &Bubble| f64::from(b.d) / 2.0;
        let ys: Vec<(f64, f64)> = bubbles.iter().map(|b| (b.y, radius(b))).collect();
        let vs = fit_scale((dlo, dhi), zero, (vmin, vmax), &ys, f64::from(h));
        let vticks = vs.ticks().into_iter().map(|t| (t, format_value(t, &c.val_fmt, vs.step))).collect();
        let xs = c.kind.xy().then(|| {
            let (xl, xh) = x_extent(c);
            let xs: Vec<(f64, f64)> = bubbles.iter().map(|b| (b.x, radius(b))).collect();
            fit_scale((xl, xh), bubble && zero_for(xl, xh), (c.cat.min, c.cat.max), &xs, f64::from(w))
        });
        // Scatter x values show in the axis's own format, else their cache's (dates included).
        let xfmt = c.cat.fmt.clone().or_else(|| c.series.iter().find_map(|s| s.x_fmt.clone())).unwrap_or_else(|| "General".to_string());
        let xticks = xs.map(|s| s.ticks().into_iter().map(|t| (t, category_text(t, &xfmt, s.step, c.date1904))).collect()).unwrap_or_default();
        Axes { vs, vticks, xs, xticks }
    }
}

/// [`nice_scale`] for data `dlo..dhi`, widened until every disc (centre in axis units, radius in
/// points) fits whole on an axis `len` points long: first the least room that fits them at the
/// data's own span, then, while one still sticks out of the rounded scale, room at that scale's
/// span. An end the file fixes (`min`, `max`) stays put; the drawing cuts what crosses it.
fn fit_scale((dlo, dhi): (f64, f64), zero: bool, (min, max): (Option<f64>, Option<f64>), discs: &[(f64, f64)], len: f64) -> Scale {
    const ROUNDS: usize = 8;
    let plain = nice_scale(dlo, dhi, zero, min, max);
    if discs.is_empty() || !(len.is_finite() && len > 0.0) {
        return plain;
    }
    // The ends every disc needs when the axis spans `span` units.
    let room = |span: f64| {
        let k = span / len;
        discs.iter().fold((dlo, dhi), |(lo, hi), (v, r)| (lo.min(v - r * k), hi.max(v + r * k)))
    };
    let finite = |(l, h): (f64, f64)| l.is_finite() && h.is_finite();
    let (mut lo, mut hi) = (dlo, dhi);
    for _ in 0..ROUNDS {
        let next = room(hi - lo);
        if !finite(next) {
            return plain;
        }
        (lo, hi) = next;
    }
    let mut s = nice_scale(lo, hi, zero, min, max);
    for _ in 0..ROUNDS {
        let (l, h) = room(s.hi - s.lo);
        if !finite((l, h)) || ((min.is_some() || l >= s.lo) && (max.is_some() || h <= s.hi)) {
            break;
        }
        (lo, hi) = (lo.min(l), hi.max(h));
        s = nice_scale(lo, hi, zero, min, max);
    }
    s
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
    let scatter = c.kind.xy();
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

/// A bar, column, line, area, scatter or bubble chart in `area`: gridlines, series, axes, then labels.
pub(super) fn cartesian(out: &mut Vec<GraphicItem>, c: &Chart, area: Rect) {
    if c.n == 0 {
        return;
    }
    let inner = |m: &Margins| Rect { x: area.x + m.left, y: area.y + m.top, w: area.w - m.left, h: area.h - m.top - m.bottom };
    let bubbles = if c.kind == Kind::Bubble { bubble_list(c, area) } else { Vec::new() };
    let mut ax = Axes::of(c);
    let mut m = axis_margins(c, &ax, area);
    // Room for the bubbles at this plot's size, then the margins the new tick labels need, until
    // the plot keeps its size (a few rounds; what still crosses the plot's edge is cut).
    let mut fitted = (0.0, 0.0);
    for _ in 0..if bubbles.is_empty() { 0 } else { 4 } {
        let p = inner(&m);
        if (p.w, p.h) == fitted {
            break;
        }
        fitted = (p.w, p.h);
        ax = Axes::fitting(c, &bubbles, p.w, p.h);
        m = axis_margins(c, &ax, area);
    }
    let plot = inner(&m);
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
        Kind::Bubble => draw_bubbles(out, c, &g, &bubbles, &mut labels),
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
    if c.kind.xy() {
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

/// How far inside the chart's edge Word measures a bubble chart's size from, points.
const BUBBLE_INSET: f32 = 5.0;
/// How many plot areas a chart's bubbles may cover, all told.
pub(super) const BUBBLE_COVER: f32 = 256.0;
/// How many corners a chart's cut bubbles may have, all told.
const MAX_CUT_CORNERS: usize = 200_000;
/// Width of a bubble's outline, points: Word draws it this thin whatever the line's `w` (measured
/// in its PDF export with 1.5 and 3 pt lines).
const BUBBLE_OUTLINE: f32 = 0.25;

/// A bubble to draw: point `i` of series `j`, centred on (`x`, `y`) in axis units, of size `v`
/// and `d` points across.
pub(super) struct Bubble {
    j: usize,
    i: usize,
    x: f64,
    y: f64,
    v: f64,
    d: f32,
}

/// The bubbles of `c` in drawing order, sized as Word sizes bubbles (measured in Word for
/// Microsoft 365). The largest size of the chart gets a diameter of `L * 3s / (3s + 1000)`, where
/// `L` is the short side of the room the title and legend leave (the chart less 5 pt a side when
/// it has neither, `area` being [`PAD`] inside its edge) and `s` is the `c:bubbleScale`. Other
/// bubbles scale from the largest by area, or by width with `c:sizeRepresents="w"`. A size of 0
/// isn't drawn, nor a negative one unless `c:showNegBubbles` is on (then at its magnitude). Points
/// without a size, an x or a y are skipped, and don't count towards the largest (Word, measured
/// with a size-64 point missing its x or its y). A series with no `c:bubbleSize` at all sizes every
/// point 1 (not checked against Word).
fn bubble_list(c: &Chart, area: Rect) -> Vec<Bubble> {
    let Some(opts) = c.bubble else { return Vec::new() };
    let size = |s: &Series, i: usize| if s.sizes.is_empty() { Some(1.0) } else { s.sizes.get(i).copied().flatten() };
    let shown = |v: f64| v > 0.0 || (opts.negative && v < 0.0);
    let mut out = Vec::new();
    for (j, s) in c.series.iter().enumerate() {
        for (i, (x, y)) in s.xs.iter().zip(&s.values).enumerate() {
            if let (Some(x), Some(y), Some(v)) = (*x, *y, size(s, i))
                && x.is_finite()
                && y.is_finite()
                && v.is_finite()
                && shown(v)
            {
                out.push(Bubble { j, i, x, y, v, d: 0.0 });
            }
        }
    }
    let largest = out.iter().map(|b| b.v.abs()).fold(0.0, f64::max);
    let side = f64::from((area.w.min(area.h) + 2.0 * (PAD - BUBBLE_INSET)).max(0.0));
    let d_max = side * 3.0 * opts.scale / (3.0 * opts.scale + 1000.0);
    if !(largest > 0.0 && largest.is_finite() && d_max > 0.0 && d_max.is_finite()) {
        return Vec::new();
    }
    for b in &mut out {
        let share = b.v.abs() / largest;
        b.d = (d_max * if opts.by_width { share } else { share.sqrt() }) as f32;
    }
    out.retain(|b| b.d.is_finite() && b.d > 0.0);
    out
}

/// Bubble series: a circle per point in [`bubble_list`], centred on its x and y.
///
/// Word keeps bubbles inside the plot area: automatic axes leave room for them (see
/// [`Axes::fitting`]), a bubble crossing an end the file fixes is cut at the plot's edge, and a
/// point outside a fixed range isn't drawn (all measured in Word's PDF export). A cut bubble is a
/// path, not an ellipse. Drawing stops once the bubbles cover [`BUBBLE_COVER`] plot areas, which
/// bounds the overdraw of a hostile file (100,000 points, each up to half the chart's short side),
/// and bubbles stop being cut after [`MAX_CUT_CORNERS`].
///
/// A bubble is outlined when its point or series has a visible line. A negative bubble is drawn
/// inverted, as Office always does (MS-OI29500 §21.2.2.86, whatever `c:invertIfNegative` says):
/// white, outlined in its line colour or else black.
fn draw_bubbles(out: &mut Vec<GraphicItem>, c: &Chart, g: &Frame, bubbles: &[Bubble], labels: &mut Vec<DataLabel>) {
    let Some(xs) = g.xs else { return };
    let plot = g.plot;
    let budget = f64::from(BUBBLE_COVER) * f64::from(plot.w) * f64::from(plot.h);
    let (mut covered, mut corners) = (0.0, 0);
    for b in bubbles {
        if b.x < xs.lo || b.x > xs.hi || b.y < g.vs.lo || b.y > g.vs.hi {
            continue;
        }
        covered += std::f64::consts::FRAC_PI_4 * f64::from(b.d) * f64::from(b.d);
        if covered > budget {
            break;
        }
        let Some(s) = c.series.get(b.j) else { continue };
        let (cx, cy, d) = (g.scat_x(b.x), g.val(b.y), b.d);
        let line = s.outlines.get(b.i).copied().flatten();
        let (fill, stroke) = if b.v < 0.0 { (Rgb::WHITE, Some(line.unwrap_or(Rgb::BLACK))) } else { (c.point_colour(b.j, b.i), line) };
        let r = d / 2.0;
        if cx - r >= plot.x && cy - r >= plot.y && cx + r <= plot.right() && cy + r <= plot.bottom() {
            let stroke_width = if stroke.is_some() { BUBBLE_OUTLINE } else { 0.0 };
            out.push(GraphicItem::Shape { rect: [cx - r, cy - r, d, d], kind: ShapeKind::Ellipse, fill: Some(fill), stroke, stroke_width });
        } else {
            let ring = circle(cx, cy, r);
            corners += ring.len();
            if corners > MAX_CUT_CORNERS {
                continue;
            }
            let cut = clip_polygon(&ring, plot);
            if cut.len() < 3 {
                continue;
            }
            fill_poly(out, &cut, fill);
            if let Some(col) = stroke {
                super::canvas::stroke(out, clip_ring(&ring, plot), col, BUBBLE_OUTLINE);
            }
        }
        if let Some(text) = label_of(s, b.i) {
            labels.push(DataLabel { x: cx, y: (cy - r).max(plot.y), text, inside: false });
        }
    }
}

/// A circle as a closed polygon, with enough corners that it strays under 0.1 pt from the circle.
fn circle(cx: f32, cy: f32, r: f32) -> Vec<(f32, f32)> {
    let n = ((r.max(0.0).sqrt() * 8.0).ceil() as usize).clamp(8, 256);
    (0..n)
        .map(|k| {
            let t = k as f32 / n as f32 * std::f32::consts::TAU;
            (cx + r * t.cos(), cy + r * t.sin())
        })
        .collect()
}

/// The part of the convex polygon `poly` inside `rect` (Sutherland-Hodgman).
fn clip_polygon(poly: &[(f32, f32)], rect: Rect) -> Vec<(f32, f32)> {
    // How far a point is inside each edge (negative: outside).
    let edges: [fn((f32, f32), Rect) -> f32; 4] = [|p, r| p.0 - r.x, |p, r| r.right() - p.0, |p, r| p.1 - r.y, |p, r| r.bottom() - p.1];
    let mut pts = poly.to_vec();
    for inside in edges {
        let Some(&last) = pts.last() else { break };
        let mut next = Vec::with_capacity(pts.len() + 4);
        let mut prev = last;
        for &p in &pts {
            let (a, b) = (inside(prev, rect), inside(p, rect));
            if (a >= 0.0) != (b >= 0.0) {
                let t = a / (a - b);
                next.push((prev.0 + t * (p.0 - prev.0), prev.1 + t * (p.1 - prev.1)));
            }
            if b >= 0.0 {
                next.push(p);
            }
            prev = p;
        }
        pts = next;
    }
    pts
}

/// The parts of the closed polygon `ring`'s outline inside `rect`, as open paths.
fn clip_ring(ring: &[(f32, f32)], rect: Rect) -> Vec<PathSeg> {
    let mut segs = Vec::new();
    let mut pen: Option<(f32, f32)> = None;
    for (k, &p) in ring.iter().enumerate() {
        let q = ring.get(k + 1).or(ring.first()).copied().unwrap_or(p);
        let Some((a, b)) = clip_segment(p, q, rect) else {
            pen = None;
            continue;
        };
        if pen != Some(a) {
            segs.push(PathSeg::Move(a.0, a.1));
        }
        segs.push(PathSeg::Line(b.0, b.1));
        pen = Some(b);
    }
    segs
}

/// The part of the segment `p`..`q` inside `rect` (Liang-Barsky), if any.
fn clip_segment(p: (f32, f32), q: (f32, f32), rect: Rect) -> Option<((f32, f32), (f32, f32))> {
    let (dx, dy) = (q.0 - p.0, q.1 - p.1);
    let (mut t0, mut t1) = (0.0f32, 1.0f32);
    for (den, num) in [(-dx, p.0 - rect.x), (dx, rect.right() - p.0), (-dy, p.1 - rect.y), (dy, rect.bottom() - p.1)] {
        if den == 0.0 {
            if num < 0.0 {
                return None;
            }
            continue;
        }
        let t = num / den;
        if den < 0.0 {
            t0 = t0.max(t);
        } else {
            t1 = t1.min(t);
        }
    }
    (t0 <= t1).then_some(((p.0 + t0 * dx, p.1 + t0 * dy), (p.0 + t1 * dx, p.1 + t1 * dy)))
}

/// A filled polygon.
fn fill_poly(out: &mut Vec<GraphicItem>, pts: &[(f32, f32)], color: Rgb) {
    let mut segs: Vec<PathSeg> = pts.iter().enumerate().map(|(k, &(x, y))| if k == 0 { PathSeg::Move(x, y) } else { PathSeg::Line(x, y) }).collect();
    segs.push(PathSeg::Close);
    fill(out, segs, color);
}
