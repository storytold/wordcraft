//! Hi-low lines (`c:hiLowLines`, ECMA-376 Part 1 §21.2.2.80) and up/down bars (`c:upDownBars`,
//! §21.2.2.218) of stock and line charts.

use super::canvas::{rect_segs, seg, stroke};
use super::model::{BarStyle, Group, Stock, UpDown};
use super::plot::Frame;
use super::{El, GraphicItem, Rgb, TEXT, fill_colour, int, measure};

/// Default width of hi-low lines and up/down bar outlines, points.
const THIN: f32 = 0.75;

impl Stock {
    pub(super) fn parse(ty: &El, theme: &[Rgb]) -> Stock {
        let hi_low = ty.child("c:hiLowLines").and_then(|h| outline(h.child("c:spPr"), theme));
        let up_down = ty.child("c:upDownBars").map(|u| UpDown {
            gap: u.child("c:gapWidth").and_then(|g| g.attr("val")).and_then(int).unwrap_or(150).clamp(0, 500) as f32 / 100.0,
            up: bar_style(u.child("c:upBars"), Rgb::WHITE, theme),
            down: bar_style(u.child("c:downBars"), Rgb::BLACK, theme),
        });
        Stock { hi_low, up_down }
    }
}

/// The line of a `c:spPr` (`a:ln`): its solid colour, else the axis-text colour, and its width;
/// none for `a:noFill`.
fn outline(sp: Option<&El>, theme: &[Rgb]) -> Option<(Rgb, f32)> {
    let ln = sp.and_then(|p| p.child("a:ln"));
    if ln.is_some_and(|l| l.child("a:noFill").is_some()) {
        return None;
    }
    let color = ln.and_then(|l| fill_colour(l, theme)).unwrap_or(TEXT);
    let w = ln.and_then(|l| l.attr("w")).and_then(|v| measure(v, 12_700.0)).map_or(THIN, |w| w.clamp(0.0, 20.0));
    Some((color, w))
}

fn bar_style(el: Option<&El>, default: Rgb, theme: &[Rgb]) -> BarStyle {
    let sp = el.and_then(|e| e.child("c:spPr"));
    let fill = match sp {
        Some(p) if p.child("a:noFill").is_some() => None,
        Some(p) => fill_colour(p, theme).or(Some(default)),
        None => Some(default),
    };
    BarStyle { fill, outline: outline(sp, theme) }
}

/// A group's hi-low lines (from the highest to the lowest value of each category, all series),
/// then its up/down bars (from the first series' value to the last's) on top.
pub(super) fn draw_stock(out: &mut Vec<GraphicItem>, grp: &Group, f: &Frame) {
    let value = |s: usize, i: usize| grp.series.get(s).and_then(|s| s.values.get(i).copied().flatten());
    if let Some((color, w)) = grp.stock.hi_low {
        for i in 0..f.n {
            let vals: Vec<f64> = (0..grp.series.len()).filter_map(|s| value(s, i)).collect();
            if vals.len() < 2 {
                continue;
            }
            let hi = vals.iter().copied().fold(f64::NEG_INFINITY, f64::max);
            let lo = vals.iter().copied().fold(f64::INFINITY, f64::min);
            let x = f.cat_x(i);
            stroke(out, seg(x, f.val(hi), x, f.val(lo)), color, w);
        }
    }
    let Some(ud) = &grp.stock.up_down else { return };
    let last = grp.series.len().saturating_sub(1);
    if last == 0 {
        return;
    }
    let width = f.band() / (1.0 + ud.gap);
    for i in 0..f.n {
        let (Some(open), Some(close)) = (value(0, i), value(last, i)) else { continue };
        let style = if close >= open { &ud.up } else { &ud.down };
        let x = f.cat_x(i) - width / 2.0;
        let (y0, y1) = (f.val(open), f.val(close));
        let (stroke, stroke_width) = style.outline.map_or((None, 0.0), |(c, w)| (Some(c), w));
        if style.fill.is_some() || stroke.is_some() {
            let segs = rect_segs(x, y0.min(y1), x + width, y0.max(y1));
            out.push(GraphicItem::Path { segs, fill: style.fill, stroke, stroke_width });
        }
    }
}
