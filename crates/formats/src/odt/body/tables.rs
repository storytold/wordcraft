//! Tables being read (ODF 1.3 §9.1): rows, cells and their shared edges.

use super::{Borders, Cell, FTable};

/// Give each edge two cells share the line either cell draws: LibreOffice gives a shared edge to
/// one of them only (the left cell's right side is `none`, the right cell's left side draws it),
/// while WordCraft's cells each draw their own four sides. The cell below is the one in the next
/// row starting at the same column.
pub(in crate::odt) fn share_edges(rows: &mut [Vec<Cell>]) {
    let visible = |b: Option<wordcraft_doc::props::Border>| b.filter(|b| b.is_visible());
    for r in 0..rows.len() {
        let starts: Vec<usize> = rows
            .get(r)
            .map(|row| {
                row.iter()
                    .scan(0usize, |g, c| {
                        let s = *g;
                        *g += c.colspan.max(1) as usize;
                        Some(s)
                    })
                    .collect()
            })
            .unwrap_or_default();
        for k in 0..starts.len() {
            // The cell to the right.
            if let Some(row) = rows.get_mut(r)
                && let [Some(a), Some(b)] = [k, k + 1].map(|i| row.get(i).map(|c| c.borders))
                && (a.is_some() || b.is_some())
            {
                let (a, b) = (a.unwrap_or_default(), b.unwrap_or_default());
                let line = visible(a.right).or(visible(b.left));
                if let Some(line) = line {
                    if let Some(c) = row.get_mut(k) {
                        c.borders = Some(Borders { right: Some(line), ..a });
                    }
                    if let Some(c) = row.get_mut(k + 1) {
                        c.borders = Some(Borders { left: Some(line), ..b });
                    }
                }
            }
            // The cell below.
            let start = starts.get(k).copied().unwrap_or(0);
            let below = rows.get(r + 1).and_then(|row| {
                let mut g = 0;
                row.iter().position(|c| {
                    let s = g;
                    g += c.colspan.max(1) as usize;
                    s == start
                })
            });
            let (Some(j), Some(a)) = (below, rows.get(r).and_then(|row| row.get(k)).map(|c| c.borders)) else { continue };
            let Some(b) = rows.get(r + 1).and_then(|row| row.get(j)).map(|c| c.borders) else { continue };
            if a.is_none() && b.is_none() {
                continue;
            }
            let (a, b) = (a.unwrap_or_default(), b.unwrap_or_default());
            if let Some(line) = visible(a.bottom).or(visible(b.top)) {
                if let Some(c) = rows.get_mut(r).and_then(|row| row.get_mut(k)) {
                    c.borders = Some(Borders { bottom: Some(line), ..a });
                }
                if let Some(c) = rows.get_mut(r + 1).and_then(|row| row.get_mut(j)) {
                    c.borders = Some(Borders { top: Some(line), ..b });
                }
            }
        }
    }
}

pub(in crate::odt) struct TableB {
    pub(in crate::odt) rows: Vec<Vec<Cell>>,
    pub(in crate::odt) row: Vec<Cell>,
    /// Covered cells still owed to the previous cell's column span.
    pub(in crate::odt) hcover: usize,
    /// Widths in points of the `table:table-column`s so far, 0 where the style gives none.
    pub(in crate::odt) widths: Vec<f32>,
    /// Its alignment and width, and each row's height so far (see [`crate::model::TableExtra`]).
    pub(in crate::odt) extra: crate::model::TableExtra,
}

impl TableB {
    pub(in crate::odt) fn finish(self) -> FTable {
        // A column without a width leaves the whole grid to the equal split.
        let widths = if self.widths.iter().all(|w| *w > 0.0) { self.widths } else { Vec::new() };
        let mut rows = self.rows;
        share_edges(&mut rows);
        // A table whose cells all say "no line" draws none.
        let stated = rows.iter().flatten().filter(|c| !c.covered).all(|c| c.borders.is_some());
        let borderless = stated && rows.iter().flatten().all(|c| !c.borders.is_some_and(|b| b.any_visible()));
        let extra = self.extra;
        let kept = extra.fixed || extra.align.is_some() || extra.width_pct.is_some() || extra.heights.iter().any(Option::is_some);
        FTable { rows, widths, borderless, extra: kept.then(|| Box::new(extra)), kept: None, wrap: None }
    }
}
