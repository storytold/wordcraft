use super::ParaLayout;
use super::model::{ClKind, Line, VisCl};

/// The bidi levels a piece of text is shaped by.
#[derive(Clone, Copy)]
pub(super) enum Dir<'l> {
    /// Paragraph text: the paragraph's levels at the piece's offset.
    Para,
    /// Text laid out on its own (a field result): these levels, by byte of the text.
    Own(&'l [u8]),
    /// Each strong run in its own direction (list labels, ruby, left-to-right results).
    Guess,
}

/// The direction of `c` if it is a strong letter (UAX #9 P2): `Some(true)` right to left.
pub(super) fn strong_rtl(c: char) -> Option<bool> {
    use unicode_bidi::BidiClass::{AL, L, R};
    // In ASCII only the letters are strong (L).
    if c.is_ascii() {
        return c.is_ascii_alphabetic().then_some(false);
    }
    match unicode_bidi::bidi_class(c) {
        R | AL => Some(true),
        L => Some(false),
        _ => None,
    }
}

/// Bidi embedding levels per byte of `text` (UAX #9 rules up to I2), or empty when the
/// paragraph is left to right and has no right-to-left characters.
pub fn bidi_levels(text: &str, rtl_para: bool) -> Vec<u8> {
    if !rtl_para && !text.chars().any(wordcraft_fonts::is_rtl) {
        return Vec::new();
    }
    let level = if rtl_para { unicode_bidi::Level::rtl() } else { unicode_bidi::Level::ltr() };
    let info = unicode_bidi::ParagraphBidiInfo::new(text, Some(level));
    info.levels.iter().map(|l| l.number()).collect()
}

/// Rule L2 of UAX #9: the visual order (left to right) of items with these levels: from the
/// highest level down to the lowest odd one, reverse every run at that level or above.
pub fn visual_order(levels: &[u8]) -> Vec<usize> {
    let mut order: Vec<usize> = (0..levels.len()).collect();
    let Some(min_odd) = levels.iter().copied().filter(|l| l % 2 == 1).min() else { return order };
    let max = levels.iter().copied().max().unwrap_or(0);
    let level_at = |order: &[usize], i: usize| order.get(i).and_then(|&o| levels.get(o)).copied().unwrap_or(0);
    let mut lvl = max;
    while lvl >= min_odd {
        let mut i = 0;
        while i < order.len() {
            if level_at(&order, i) >= lvl {
                let mut j = i;
                while j < order.len() && level_at(&order, j) >= lvl {
                    j += 1;
                }
                if let Some(run) = order.get_mut(i..j) {
                    run.reverse();
                }
                i = j;
            } else {
                i += 1;
            }
        }
        if lvl == 0 {
            break;
        }
        lvl -= 1;
    }
    order
}

/// Make a row's lines as tall as its tallest, and return that height.
pub(super) fn even_row(row: &mut [Line]) -> f32 {
    let h = row.iter().map(|l| l.height).fold(0.0f32, f32::max);
    for l in row {
        l.baseline += h - l.height;
        l.height = h;
    }
    h
}

/// Finish a broken line of a bidirectional paragraph: resolve each cluster's level for the line
/// (UAX #9 rule L1), order the clusters visually (rule L2) and give each its box. `line.xs`
/// arrive in logical order measured from the paragraph's start edge (mirrored for a
/// right-to-left paragraph); afterwards `line.vis` holds the boxes and `xs` their left edges.
pub(super) fn reorder_line(pl: &ParaLayout, line: &mut Line, width: f32) {
    let rtl_para = line.rtl;
    if rtl_para {
        let (l, r) = (line.left, line.right);
        line.left = width - r;
        line.right = width - l;
    }
    let (c0, c1) = (line.c0, line.c1);
    let n = c1.saturating_sub(c0);
    if n == 0 {
        if rtl_para {
            line.xs = line.xs.iter().map(|x| width - x).collect();
        }
        return;
    }
    let para_level = u8::from(rtl_para);
    let mut levels: Vec<u8> = (c0..c1).map(|k| pl.clusters.get(k).and_then(|c| pl.bidi_levels.get(c.start)).copied().unwrap_or(para_level)).collect();
    // L1: tabs and breaks, and the whitespace before them or at the end of the line, take the
    // paragraph level (so trailing spaces stay at the line's end edge).
    let mut trailing = true;
    for (i, k) in (c0..c1).enumerate().rev() {
        let kind = pl.clusters.get(k).map(|c| c.kind);
        let reset = match kind {
            Some(ClKind::Tab | ClKind::LineBreak) => {
                trailing = true;
                true
            }
            Some(ClKind::Space | ClKind::PageBreak | ClKind::ColumnBreak | ClKind::Marker) => trailing,
            _ => {
                trailing = false;
                false
            }
        };
        if reset && let Some(v) = levels.get_mut(i) {
            *v = para_level;
        }
    }
    if !rtl_para && levels.iter().all(|l| *l == 0) {
        return; // A left-to-right line of a paragraph with right-to-left text elsewhere.
    }
    let order = visual_order(&levels);
    let hyph = line.hyphen.map_or(0.0, |h| h.2);
    let mut w: Vec<f32> = (0..n).map(|i| line.xs.get(i + 1).zip(line.xs.get(i)).map_or(0.0, |(b, a)| b - a)).collect();
    if let Some(last) = w.last_mut() {
        *last -= hyph;
    }
    let total: f32 = w.iter().sum();
    let x0 = line.xs.first().copied().unwrap_or(0.0);
    let mut x = if rtl_para { width - x0 - total } else { x0 };
    let mut vis = vec![VisCl { x: 0.0, w: 0.0, level: para_level }; n];
    for &i in &order {
        if let (Some(v), Some(wi), Some(l)) = (vis.get_mut(i), w.get(i), levels.get(i)) {
            *v = VisCl { x, w: *wi, level: *l };
            x += *wi;
        }
    }
    line.vis = vis;
    let end = line.end_x();
    line.xs = line.vis.iter().map(|v| v.x).chain(std::iter::once(end)).collect();
}
