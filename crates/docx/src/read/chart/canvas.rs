use super::model::Pos;
use super::{CHAR_W, GAP, GraphicItem, LEGEND_SHARE, LINE, MAX_LEGEND_ROWS, PAD, PathSeg, Rgb, ShapeKind, TEXT, TextAlign};

#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct Rect {
    pub(super) x: f32,
    pub(super) y: f32,
    pub(super) w: f32,
    pub(super) h: f32,
}

impl Rect {
    pub(super) fn right(self) -> f32 {
        self.x + self.w
    }
    pub(super) fn bottom(self) -> f32 {
        self.y + self.h
    }
    pub(super) fn arr(self) -> [f32; 4] {
        [self.x, self.y, self.w, self.h]
    }
}

/// Estimated text width, points: text is placed before it's shaped, at [`CHAR_W`] a character.
pub(in super::super) fn text_w(s: &str, size: f32) -> f32 {
    s.chars().count() as f32 * size * CHAR_W
}

/// `s`, cut short with an ellipsis to fit `width` points by the [`text_w`] estimate.
pub(super) fn clip_text(s: &str, width: f32, size: f32) -> String {
    let fit = (width / (size * CHAR_W)).floor().max(0.0) as usize;
    if s.chars().count() <= fit {
        return s.to_string();
    }
    let mut out: String = s.chars().take(fit.saturating_sub(1)).collect();
    out.push('…');
    out
}

pub(super) fn widest(items: &[String], size: f32) -> f32 {
    items.iter().map(|s| text_w(s, size)).fold(0.0, f32::max)
}

pub(super) fn text(out: &mut Vec<GraphicItem>, rect: Rect, s: &str, size: f32, align: TextAlign, bold: bool) {
    out.push(GraphicItem::Text { rect: rect.arr(), text: s.to_string(), size, color: TEXT, bold, align, font: None });
}

/// Text centred on `cx` with its top at `y`.
pub(super) fn text_at(out: &mut Vec<GraphicItem>, cx: f32, y: f32, s: &str, size: f32) {
    let w = text_w(s, size) + 4.0;
    text(out, Rect { x: cx - w / 2.0, y, w, h: size * LINE }, s, size, TextAlign::Center, false);
}

pub(super) fn fill(out: &mut Vec<GraphicItem>, segs: Vec<PathSeg>, color: Rgb) {
    if !segs.is_empty() {
        out.push(GraphicItem::Path { segs, fill: Some(color), stroke: None, stroke_width: 0.0 });
    }
}

pub(super) fn stroke(out: &mut Vec<GraphicItem>, segs: Vec<PathSeg>, color: Rgb, width: f32) {
    if !segs.is_empty() {
        out.push(GraphicItem::Path { segs, fill: None, stroke: Some(color), stroke_width: width });
    }
}

fn swatch(out: &mut Vec<GraphicItem>, x: f32, y: f32, color: Rgb) {
    out.push(GraphicItem::Shape { rect: [x, y, SWATCH, SWATCH], kind: ShapeKind::Rectangle, fill: Some(color), stroke: None, stroke_width: 0.0 });
}

pub(super) fn seg(x0: f32, y0: f32, x1: f32, y1: f32) -> Vec<PathSeg> {
    vec![PathSeg::Move(x0, y0), PathSeg::Line(x1, y1)]
}

pub(super) fn rect_segs(x0: f32, y0: f32, x1: f32, y1: f32) -> Vec<PathSeg> {
    vec![PathSeg::Move(x0, y0), PathSeg::Line(x1, y0), PathSeg::Line(x1, y1), PathSeg::Line(x0, y1), PathSeg::Close]
}

/// Polyline pieces: a gap (`None`) starts a new sub-path.
pub(super) fn runs(pts: impl Iterator<Item = Option<(f32, f32)>>) -> Vec<PathSeg> {
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
pub(super) fn wrap(s: &str, width: f32, size: f32) -> Vec<String> {
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

pub(super) const SWATCH: f32 = 7.0;

/// Space between legend entries in a row, points.
const SEP: f32 = 10.0;

/// Legend in the given side of `area`; returns the area left for the plot.
pub(super) fn legend(out: &mut Vec<GraphicItem>, entries: &[(String, Rgb)], pos: Pos, area: Rect, fs: f32) -> Rect {
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
