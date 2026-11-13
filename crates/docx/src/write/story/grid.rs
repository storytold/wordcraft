//! The column grid a table is saved with (`w:tblGrid`, ECMA-376 Part 1 §17.4.48) and the
//! `w:gridSpan` (§17.4.17) of each cell on it.
//!
//! A table read with a grid shorter than its widest row (a `w:tblGrid` listing fewer `w:gridCol`
//! than a row has cells, or rows carrying their own cell edges) gets a real grid on save: from the
//! union of its rows' cell edges when every cell has a fixed width, else the missing columns are
//! sized from the cells, the table width or the known columns, never from a fixed filler.

use wordcraft_doc::table::{MAX_COLS, Table};

/// Edges closer than this (points, one twip) are one edge.
const SAME_EDGE: f32 = 0.05;
/// Width of a missing column when the table says nothing about its widths.
const FALLBACK_COL: f32 = 72.0;

/// The grid widths (points) to save `t` with and, per row of `t.rows`, the `w:gridSpan` of each
/// cell. A table whose grid covers all its rows is returned as it is.
pub(super) fn saved_grid(t: &Table) -> (Vec<f32>, Vec<Vec<u32>>) {
    let cols = t.cols().clamp(1, MAX_COLS);
    let model_spans = || t.rows.iter().map(|r| r.cells.iter().map(|c| c.props.span).collect()).collect();
    if t.grid.len() >= t.cols() {
        let grid = (0..cols).map(|g| t.grid.get(g).copied().unwrap_or(FALLBACK_COL)).collect();
        return (grid, model_spans());
    }
    // A grid of edges with fewer columns than the table would undo a merge whose interior edge
    // no row draws (a lone two-column cell): keep the model's columns and spans instead.
    if let Some(regrid) = from_edges(t).filter(|(grid, _)| grid.len() >= cols) {
        return regrid;
    }
    (filled(t, cols), model_spans())
}

/// A grid of all rows' cell edges, when no row skips grid columns and every cell has a fixed
/// width; `None` otherwise or when it would need more than [`MAX_COLS`] columns.
fn from_edges(t: &Table) -> Option<(Vec<f32>, Vec<Vec<u32>>)> {
    let mut rows: Vec<Vec<f32>> = Vec::with_capacity(t.rows.len());
    for r in &t.rows {
        if r.props.grid_before > 0 || r.props.grid_after > 0 || r.cells.len() > MAX_COLS {
            return None;
        }
        let mut edges = vec![0.0f32];
        let mut x = 0.0f32;
        for c in &r.cells {
            let w = c.props.width.filter(|w| w.is_finite() && *w > 0.0)?;
            x += w;
            if !x.is_finite() {
                return None;
            }
            edges.push(x);
        }
        rows.push(edges);
    }
    let mut all: Vec<f32> = rows.iter().flatten().copied().collect();
    all.sort_by(f32::total_cmp);
    let mut merged: Vec<f32> = Vec::new();
    for e in all {
        if merged.last().is_none_or(|last| e - last >= SAME_EDGE) {
            merged.push(e);
            if merged.len() > MAX_COLS + 1 {
                return None;
            }
        }
    }
    let grid: Vec<f32> = merged.windows(2).filter_map(|p| Some(p.get(1)? - p.first()?)).collect();
    if grid.is_empty() {
        return None;
    }
    let mut spans = Vec::with_capacity(rows.len());
    for edges in &rows {
        let at: Vec<usize> = edges.iter().map(|e| nearest(&merged, *e)).collect();
        let mut row = Vec::with_capacity(at.len());
        for p in at.windows(2) {
            let span = p.get(1)?.checked_sub(*p.first()?).filter(|s| *s > 0)?;
            row.push(u32::try_from(span).ok()?);
        }
        spans.push(row);
    }
    Some((grid, spans))
}

/// Index of the edge in sorted `edges` nearest `e`.
fn nearest(edges: &[f32], e: f32) -> usize {
    let i = edges.partition_point(|x| *x < e);
    let dist = |j: usize| edges.get(j).map_or(f32::INFINITY, |x| (x - e).abs());
    if i > 0 && dist(i - 1) <= dist(i) { i - 1 } else { i }
}

/// The table's own grid, with each missing column `g` sized from the first cell covering it that
/// has a fixed width; the others share what the table width leaves after the known and the
/// cell-sized columns, else take the mean known column.
fn filled(t: &Table, cols: usize) -> Vec<f32> {
    let known = t.grid.len().min(cols);
    let known_sum: f32 = t.grid.iter().take(known).map(|w| w.max(0.0)).sum();
    let positive: Vec<f32> = t.grid.iter().take(known).copied().filter(|w| *w > 0.0).collect();
    let sized: Vec<Option<f32>> = (known..cols).map(|g| from_cell(t, g, cols)).collect();
    let sized_sum: f32 = sized.iter().flatten().sum();
    let unresolved = sized.iter().filter(|w| w.is_none()).count();
    let from_table = t
        .props
        .width
        .filter(|tw| tw.is_finite())
        .map(|tw| tw - known_sum - sized_sum)
        .filter(|left| *left > 0.0)
        .map(|left| left / unresolved.max(1) as f32);
    let mean = (!positive.is_empty()).then(|| positive.iter().sum::<f32>() / positive.len() as f32);
    let rest = from_table.or(mean).unwrap_or(FALLBACK_COL);
    let mut grid: Vec<f32> = t.grid.iter().take(known).copied().collect();
    grid.extend(sized.into_iter().map(|w| w.unwrap_or(rest)));
    grid
}

/// Width of missing grid column `g` from the first cell, top to bottom, covering it with a fixed
/// width: what of that width its known columns leave, shared by its missing ones.
fn from_cell(t: &Table, g: usize, cols: usize) -> Option<f32> {
    for r in &t.rows {
        let mut start = r.first_grid();
        for c in &r.cells {
            let end = start.saturating_add(c.span()).min(cols);
            if start <= g && g < end {
                if let Some(w) = c.props.width.filter(|w| w.is_finite()) {
                    let known: f32 = (start..end).filter_map(|i| t.grid.get(i)).map(|w| w.max(0.0)).sum();
                    let missing = (start..end).filter(|i| *i >= t.grid.len()).count().max(1);
                    let share = (w - known) / missing as f32;
                    if share > 0.0 {
                        return Some(share);
                    }
                }
                break;
            }
            start = end;
            if start > g {
                break;
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use wordcraft_doc::table::{Cell, Row};

    fn cell(w: Option<f32>) -> Cell {
        let mut c = Cell::with_text("x");
        c.props.width = w;
        c
    }

    fn table(grid: &[f32], rows: &[&[Option<f32>]]) -> Table {
        Table {
            grid: grid.to_vec(),
            rows: rows.iter().map(|r| Row { cells: r.iter().map(|w| cell(*w)).collect(), ..Default::default() }).collect(),
            ..Default::default()
        }
    }

    #[test]
    fn rows_with_different_edges_share_one_grid() {
        let t = table(&[450.0], &[&[Some(225.0), Some(225.0)], &[Some(150.0), Some(150.0), Some(150.0)]]);
        assert_eq!(saved_grid(&t), (vec![150.0, 75.0, 75.0, 150.0], vec![vec![2, 2], vec![1, 2, 1]]));
    }

    #[test]
    fn edges_within_a_twip_are_one_edge() {
        let t = table(&[100.0], &[&[Some(50.0), Some(50.0)], &[Some(50.03), Some(49.97), Some(30.0)]]);
        let (grid, spans) = saved_grid(&t);
        assert_eq!(grid.len(), 3);
        assert_eq!(spans, vec![vec![1, 1], vec![1, 1, 1]]);
    }

    #[test]
    fn a_row_with_grid_before_keeps_its_spans_and_fills_the_grid() {
        let mut t = table(&[100.0], &[&[Some(100.0), Some(60.0)]]);
        t.rows[0].props.grid_before = 1;
        let (grid, spans) = saved_grid(&t);
        // Column 1 is the first cell's, 2 the second's: the first covers column 1 alone.
        assert_eq!(grid, vec![100.0, 100.0, 60.0]);
        assert_eq!(spans, vec![vec![1, 1]]);
    }

    #[test]
    fn a_cell_wider_than_its_known_columns_sizes_its_missing_ones() {
        let mut t = table(&[100.0], &[&[None, None]]);
        t.rows[0].cells[0].props.span = 2;
        t.rows[0].cells[0].props.width = Some(250.0);
        t.rows[0].cells[1].props.width = None;
        let (grid, _) = saved_grid(&t);
        assert_eq!(grid, vec![100.0, 150.0, 100.0]);
    }

    #[test]
    fn no_width_information_falls_back_to_an_inch() {
        let t = table(&[], &[&[None, None]]);
        assert_eq!(saved_grid(&t).0, vec![72.0, 72.0]);
    }

    fn spanned(grid: &[f32], rows: &[&[(u32, Option<f32>)]]) -> Table {
        let mut t = table(grid, &[]);
        t.rows = rows
            .iter()
            .map(|r| Row {
                cells: r
                    .iter()
                    .map(|(span, w)| {
                        let mut c = cell(*w);
                        c.props.span = *span;
                        c
                    })
                    .collect(),
                ..Default::default()
            })
            .collect();
        t
    }

    #[test]
    fn a_lone_two_column_cell_keeps_both_columns() {
        let t = spanned(&[100.0], &[&[(2, Some(200.0))]]);
        assert_eq!(saved_grid(&t), (vec![100.0, 100.0], vec![vec![2]]));
    }

    #[test]
    fn edges_short_of_the_table_columns_keep_the_model_spans() {
        let row: &[(u32, Option<f32>)] = &[(2, Some(200.0)), (1, Some(100.0))];
        let t = spanned(&[], &[row, row]);
        assert_eq!(saved_grid(&t), (vec![100.0, 100.0, 100.0], vec![vec![2, 1], vec![2, 1]]));
    }

    #[test]
    fn edges_giving_every_table_column_are_used() {
        let t = spanned(&[100.0], &[&[(2, Some(200.0))], &[(1, Some(120.0)), (1, Some(80.0))]]);
        assert_eq!(saved_grid(&t), (vec![120.0, 80.0], vec![vec![2], vec![1, 1]]));
    }

    fn with_width(mut t: Table, w: Option<f32>) -> Table {
        t.props.width = w;
        t
    }

    #[test]
    fn unsized_columns_share_what_the_cell_sized_ones_leave() {
        let t = with_width(table(&[100.0], &[&[None, Some(250.0), None]]), Some(500.0));
        assert_eq!(saved_grid(&t), (vec![100.0, 250.0, 150.0], vec![vec![1, 1, 1]]));
        let t = with_width(table(&[100.0], &[&[None, Some(250.0), None, None]]), Some(500.0));
        assert_eq!(saved_grid(&t), (vec![100.0, 250.0, 75.0, 75.0], vec![vec![1, 1, 1, 1]]));
    }

    #[test]
    fn a_table_width_used_up_leaves_the_mean_known_column() {
        let t = with_width(table(&[100.0], &[&[None, Some(250.0), None]]), Some(300.0));
        assert_eq!(saved_grid(&t).0, vec![100.0, 250.0, 100.0]);
    }

    #[test]
    fn no_table_width_and_no_known_columns_fall_back_to_an_inch_beside_a_sized_one() {
        let t = table(&[], &[&[None, Some(60.0), None]]);
        assert_eq!(saved_grid(&t).0, vec![72.0, 60.0, 72.0]);
    }

    #[test]
    fn with_no_cell_sized_columns_the_table_width_is_shared() {
        let t = with_width(table(&[100.0], &[&[None, None, None]]), Some(500.0));
        assert_eq!(saved_grid(&t).0, vec![100.0, 200.0, 200.0]);
    }

    #[test]
    fn a_table_width_that_is_not_finite_counts_as_none() {
        for w in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
            let t = with_width(table(&[100.0], &[&[None, Some(250.0), None]]), Some(w));
            assert_eq!(saved_grid(&t).0, vec![100.0, 250.0, 100.0]);
        }
    }

    #[test]
    fn hostile_widths_never_panic() {
        let t = table(&[f32::NAN], &[&[Some(f32::INFINITY), Some(-5.0), None], &[Some(f32::MAX), Some(f32::MAX)]]);
        let (grid, spans) = saved_grid(&t);
        assert!(grid.len() <= MAX_COLS);
        assert_eq!(spans.len(), 2);
        let wide: Vec<Option<f32>> = (0..63).map(|i| Some(1.0 + i as f32 * 0.37)).collect();
        let t = table(&[1.0], &[&wide, &[Some(1000.0)], &wide[1..]]);
        assert!(saved_grid(&t).0.len() <= MAX_COLS);
    }
}
