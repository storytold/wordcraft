use super::model::Cluster;
use super::*;

/// Next tab stop after `x` (column coordinates): explicit stops, the hanging-indent implicit stop,
/// then default stops.
pub(super) fn next_tab(x: f32, tabs: &[TabStop], default_tab: f32, hanging_at: Option<f32>) -> TabStop {
    let mut best: Option<TabStop> = tabs.iter().copied().filter(|t| t.pos > x + 0.01).min_by(|a, b| a.pos.total_cmp(&b.pos));
    if let Some(h) = hanging_at
        && h > x + 0.01
        && best.is_none_or(|t| h < t.pos)
    {
        best = Some(TabStop { pos: h, align: TabAlign::Left, leader: TabLeader::None });
    }
    best.unwrap_or_else(|| {
        let d = if default_tab > 1.0 { default_tab } else { 36.0 };
        // Left of the margin Word's arithmetic truncates toward zero: the next default stop is
        // the first multiple of the interval *at or after* `x`, so a tab that already sits on
        // one (a second tab in a row, or a line starting at -72 pt) stays put. Measured on Word
        // 2016+: a line starting at -72 pt with a leading tab stays at -72, at -60 goes to -36;
        // "a\t\tX" at -60 puts X at -36, not 0. Letterheads with negative indents rely on it.
        let n = if x < -0.01 { ((x - 0.01) / d).ceil() } else { ((x + 0.01) / d).floor() + 1.0 };
        TabStop { pos: (n * d).min(1e6), align: TabAlign::Left, leader: TabLeader::None }
    })
}

/// A positional tab's stop (column coordinates): the left edge, centre or right edge of the
/// column (`margin`) or of the space between the indents (`indent`), whatever the tab stops.
pub(super) fn ptab_stop(align: PTabAlign, base: PTabBase, leader: TabLeader, width: f32, indents: (f32, f32)) -> TabStop {
    let (lo, hi) = match base {
        PTabBase::Margin => (0.0, width),
        PTabBase::Indent => (indents.0, width - indents.1),
    };
    let (pos, align) = match align {
        PTabAlign::Left => (lo, TabAlign::Left),
        PTabAlign::Center => ((lo + hi) / 2.0, TabAlign::Center),
        PTabAlign::Right => (hi, TabAlign::Right),
    };
    TabStop { pos, align, leader }
}

pub(super) fn pl_text_at(c: &Cluster) -> bool {
    c.dot
}

pub(super) fn resolve_tab(pl: &mut ParaLayout, pending: &mut Option<(usize, TabStop, f32)>, xs: &mut [f32], c0: usize, x: f32, xo: &mut f32) {
    // Finish a centre/right/decimal tab: its text (possibly empty) ends at `x`.
    let Some((tj, stop, tx)) = pending.take() else { return };
    let seg = x - tx - pl.clusters.get(tj).map(|c| c.adv).unwrap_or(0.0);
    let want = match stop.align {
        TabAlign::Right | TabAlign::Decimal => stop.pos - tx - seg,
        TabAlign::Center => stop.pos - tx - seg / 2.0,
        _ => return,
    }
    .max(0.0);
    let old = pl.clusters.get(tj).map(|c| c.adv).unwrap_or(0.0);
    let delta = want - old;
    if delta.abs() > 0.001 {
        if let Some(c) = pl.clusters.get_mut(tj) {
            c.adv = want;
        }
        for v in xs.iter_mut().skip(tj + 1 - c0) {
            *v += delta;
        }
        *xo += delta;
    }
}
