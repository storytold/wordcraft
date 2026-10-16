//! Tables: a column grid, rows and cells (cells hold block lists). Merges follow OOXML: a cell
//! spans `span` grid columns; vertical merges are `Restart` + `Continue` cells.

use serde::{Deserialize, Serialize};

use crate::control::ControlWrap;
use crate::props::{CellProps, RowProps, TableProps, VMerge};
use crate::{Blocks, Paragraph, para_block};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Default)]
pub struct Cell {
    #[serde(default)]
    pub props: CellProps,
    pub blocks: Blocks,
    /// Content controls around the cell (see [`crate::control`]).
    #[serde(default, skip_serializing_if = "ControlWrap::is_empty")]
    pub controls: ControlWrap,
}

impl Cell {
    pub fn empty() -> Cell {
        Cell { props: CellProps { span: 1, ..Default::default() }, blocks: vec![para_block(Paragraph::new())], controls: ControlWrap::default() }
    }
    pub fn with_text(s: &str) -> Cell {
        Cell {
            props: CellProps { span: 1, ..Default::default() },
            blocks: vec![para_block(Paragraph::with_text(s, Default::default()))],
            controls: ControlWrap::default(),
        }
    }
    pub fn span(&self) -> usize {
        self.props.span.clamp(1, 63) as usize
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Default)]
pub struct Row {
    #[serde(default)]
    pub props: RowProps,
    pub cells: Vec<Cell>,
    /// Content controls around the row (repeating sections; see [`crate::control`]).
    #[serde(default, skip_serializing_if = "ControlWrap::is_empty")]
    pub controls: ControlWrap,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Default)]
pub struct Table {
    #[serde(default)]
    pub props: TableProps,
    /// Grid column widths, points.
    pub grid: Vec<f32>,
    pub rows: Vec<Row>,
    /// Content controls whose content starts or ends with this table (see [`crate::control`]).
    #[serde(default, skip_serializing_if = "ControlWrap::is_empty")]
    pub controls: ControlWrap,
}

/// Hard limits that keep hostile input from exhausting memory.
pub const MAX_ROWS: usize = 32_767;
pub const MAX_COLS: usize = 63;

impl Table {
    /// A copy of rows `rows` of this table holding, of each row, the cells that reach into grid
    /// columns `cols` (both inclusive): what Copy takes from selected rows or cells. Content
    /// controls around rows and the table aren't copied (they'd lose their other end); a cell
    /// that continues a vertical merge from a row left out starts the merge instead.
    pub fn slice(&self, rows: (usize, usize), cols: (usize, usize)) -> Table {
        let ((r0, r1), (g0, g1)) = (rows, cols);
        let kept = || self.rows.iter().take(r1.saturating_add(1)).skip(r0);
        // Grid columns `lo..hi` the cells touching them cover: a cell merged across that is partly
        // in them widens them, for every row, so the copy isn't ragged. Each round only widens
        // (at most once per grid column), so a bound of the grid's width plus one round is enough.
        let (mut lo, mut hi) = (g0, g1.saturating_add(1));
        let mut any = false;
        for _ in 0..=MAX_COLS.saturating_add(1) {
            let (mut nlo, mut nhi, mut found) = (lo, hi, false);
            for row in kept() {
                let mut g = 0usize;
                for cell in &row.cells {
                    let end = g.saturating_add(cell.span());
                    if g < hi && end > lo {
                        (nlo, nhi, found) = (nlo.min(g), nhi.max(end), true);
                    }
                    g = end;
                }
            }
            any = found;
            if (nlo, nhi) == (lo, hi) {
                break;
            }
            (lo, hi) = (nlo, nhi);
        }
        let mut out_rows = Vec::new();
        for row in kept() {
            let mut g = 0usize;
            let mut cells = Vec::new();
            for cell in &row.cells {
                let end = g.saturating_add(cell.span());
                if any && g < hi && end > lo {
                    cells.push(cell.clone());
                }
                g = end;
            }
            if !cells.is_empty() {
                out_rows.push(Row { props: row.props.clone(), cells, controls: ControlWrap::default() });
            }
        }
        if let Some(first) = out_rows.first_mut() {
            for c in &mut first.cells {
                if c.props.vmerge == VMerge::Continue {
                    c.props.vmerge = VMerge::Restart;
                }
            }
        }
        let whole = lo == 0 && hi >= self.grid.len();
        let grid = if whole { self.grid.clone() } else { self.grid.get(lo..hi.min(self.grid.len())).map(<[f32]>::to_vec).unwrap_or_default() };
        let mut props = self.props.clone();
        if !whole {
            // Part of the columns: as wide as they are, not the whole table's width.
            props.width = props.width.map(|_| grid.iter().sum());
            props.width_pct = None;
        }
        Table { props, grid, rows: out_rows, controls: ControlWrap::default() }
    }

    /// A `rows × cols` table spanning `width` points, "Table Grid" style.
    pub fn new(rows: usize, cols: usize, width: f32) -> Table {
        let rows = rows.clamp(1, MAX_ROWS);
        let cols = cols.clamp(1, MAX_COLS);
        let w = (width / cols as f32).max(6.0);
        Table {
            props: TableProps { style: Some("TableGrid".into()), ..Default::default() },
            grid: vec![w; cols],
            rows: (0..rows)
                .map(|_| Row {
                    props: RowProps::default(),
                    cells: (0..cols)
                        .map(|_| {
                            let mut c = Cell::empty();
                            c.props.width = Some(w);
                            c
                        })
                        .collect(),
                    controls: ControlWrap::default(),
                })
                .collect(),
            controls: ControlWrap::default(),
        }
    }

    pub fn cols(&self) -> usize {
        self.grid.len().max(self.rows.iter().map(|r| r.cells.iter().map(Cell::span).sum::<usize>()).max().unwrap_or(0))
    }

    /// Grid column where cell `c` of row `r` starts.
    pub fn grid_col(&self, r: usize, c: usize) -> usize {
        self.rows.get(r).map(|row| row.cells.iter().take(c).map(Cell::span).sum()).unwrap_or(0)
    }

    /// Cell index in row `r` that covers grid column `g`.
    pub fn cell_at_grid(&self, r: usize, g: usize) -> Option<usize> {
        let row = self.rows.get(r)?;
        let mut x = 0;
        for (i, c) in row.cells.iter().enumerate() {
            if g < x + c.span() {
                return Some(i);
            }
            x += c.span();
        }
        None
    }

    /// Insert a row copied (formatting, empty text) from row `like` at index `at`.
    pub fn insert_row(&mut self, at: usize, like: usize) {
        let Some(src) = self.rows.get(like.min(self.rows.len().saturating_sub(1))) else { return };
        if self.rows.len() >= MAX_ROWS {
            return;
        }
        let mut row = Row { props: src.props.clone(), cells: Vec::with_capacity(src.cells.len()), controls: ControlWrap::default() };
        row.props.header = false;
        for c in &src.cells {
            let mut nc = Cell::empty();
            nc.props = c.props.clone();
            if nc.props.vmerge == VMerge::Restart {
                nc.props.vmerge = VMerge::None;
            }
            row.cells.push(nc);
        }
        self.rows.insert(at.min(self.rows.len()), row);
    }

    pub fn delete_row(&mut self, r: usize) {
        if r < self.rows.len() {
            self.rows.remove(r);
        }
    }

    /// Insert a grid column at grid index `g` (cells spanning it widen instead).
    pub fn insert_col(&mut self, g: usize) {
        if self.grid.len() >= MAX_COLS {
            return;
        }
        let g = g.min(self.grid.len());
        let w = self.grid.get(g.min(self.grid.len().saturating_sub(1))).copied().unwrap_or(72.0);
        self.grid.insert(g, w);
        for r in 0..self.rows.len() {
            let mut x = 0;
            let mut idx = None;
            if let Some(row) = self.rows.get(r) {
                for (i, c) in row.cells.iter().enumerate() {
                    if g <= x {
                        idx = Some((i, false));
                        break;
                    }
                    if g < x + c.span() {
                        idx = Some((i, true));
                        break;
                    }
                    x += c.span();
                }
            }
            if let Some(row) = self.rows.get_mut(r) {
                match idx {
                    Some((i, true)) => {
                        if let Some(c) = row.cells.get_mut(i) {
                            c.props.span += 1;
                        }
                    }
                    Some((i, false)) => {
                        let mut c = Cell::empty();
                        c.props.width = Some(w);
                        row.cells.insert(i, c);
                    }
                    None => {
                        let mut c = Cell::empty();
                        c.props.width = Some(w);
                        row.cells.push(c);
                    }
                }
            }
        }
    }

    /// Delete grid column `g` (spanning cells shrink).
    pub fn delete_col(&mut self, g: usize) {
        if g >= self.grid.len() || self.grid.len() <= 1 {
            return;
        }
        self.grid.remove(g);
        for row in &mut self.rows {
            let mut x = 0;
            let mut rm = None;
            for (i, c) in row.cells.iter_mut().enumerate() {
                if g < x + c.span() {
                    if c.span() > 1 {
                        c.props.span -= 1;
                    } else {
                        rm = Some(i);
                    }
                    break;
                }
                x += c.span();
            }
            if let Some(i) = rm {
                row.cells.remove(i);
            }
        }
        self.rows.retain(|r| !r.cells.is_empty());
    }

    /// Merge cells `c0..=c1` of row `r` horizontally (content is concatenated).
    pub fn merge_right(&mut self, r: usize, c0: usize, c1: usize) {
        let Some(row) = self.rows.get_mut(r) else { return };
        if c1 <= c0 || c1 >= row.cells.len() {
            return;
        }
        let removed: Vec<Cell> = row.cells.drain(c0 + 1..=c1).collect();
        if let Some(first) = row.cells.get_mut(c0) {
            for c in removed {
                first.props.span += c.props.span.max(1);
                let nonempty: Vec<_> = c.blocks.into_iter().filter(|b| b.as_para().is_none_or(|p| !p.is_empty())).collect();
                first.blocks.extend(nonempty);
            }
            if first.blocks.len() > 1 && first.blocks.first().and_then(|b| b.as_para()).is_some_and(Paragraph::is_empty) {
                first.blocks.remove(0);
            }
        }
    }

    /// Merge a rectangle of cells (rows r0..=r1, grid columns g0..=g1).
    pub fn merge(&mut self, r0: usize, r1: usize, g0: usize, g1: usize) {
        for r in r0..=r1.min(self.rows.len().saturating_sub(1)) {
            let (Some(a), Some(b)) = (self.cell_at_grid(r, g0), self.cell_at_grid(r, g1)) else { continue };
            self.merge_right(r, a, b);
            if let Some(c) = self.rows.get_mut(r).and_then(|row| row.cells.get_mut(a)) {
                c.props.vmerge = if r1 == r0 {
                    VMerge::None
                } else if r == r0 {
                    VMerge::Restart
                } else {
                    VMerge::Continue
                };
            }
        }
        // Move content of continued cells into the restart cell.
        if r1 > r0
            && let Some(a) = self.cell_at_grid(r0, g0)
        {
            let mut moved = Vec::new();
            for r in r0 + 1..=r1.min(self.rows.len().saturating_sub(1)) {
                if let Some(c) = self.cell_at_grid(r, g0).and_then(|ci| self.rows.get_mut(r).and_then(|row| row.cells.get_mut(ci))) {
                    let blocks = std::mem::replace(&mut c.blocks, vec![para_block(Paragraph::new())]);
                    moved.extend(blocks.into_iter().filter(|b| b.as_para().is_none_or(|p| !p.is_empty())));
                }
            }
            if let Some(c) = self.rows.get_mut(r0).and_then(|row| row.cells.get_mut(a)) {
                c.blocks.extend(moved);
            }
        }
    }

    /// Split cell `c` of row `r` into `n` cells horizontally. When the cell spans fewer grid
    /// columns than that, the grid is refined inside the cell only: the new borders divide it
    /// into equal parts, and the cells of other rows widen over the new grid columns, so their
    /// extent and the table's width stay as they were.
    pub fn split_cell(&mut self, r: usize, c: usize, n: usize) {
        let n = n.clamp(1, MAX_COLS);
        let Some(span) = self.rows.get(r).and_then(|row| row.cells.get(c)).map(Cell::span) else { return };
        if n <= 1 {
            return;
        }
        if span >= n {
            let Some(row) = self.rows.get_mut(r) else { return };
            let Some(cell) = row.cells.get_mut(c) else { return };
            let w = cell.props.width.map(|w| w / n as f32);
            cell.props.width = w;
            cell.props.span = (span - (n - 1)) as u32;
            for k in 1..n {
                let mut nc = Cell::empty();
                nc.props.width = w;
                row.cells.insert(c + k, nc);
            }
            return;
        }
        let g = self.grid_col(r, c);
        let mut end = g.saturating_add(span);
        // A grid narrower than the row (a damaged file) gets the columns the cell needs.
        while self.grid.len() < end && self.grid.len() < MAX_COLS {
            self.grid.push(72.0);
        }
        if self.grid.len() < end {
            return;
        }
        for w in self.grid.get_mut(g..end).into_iter().flatten() {
            if !(w.is_finite() && *w > 0.0) {
                *w = 72.0;
            }
        }
        const EPS: f32 = 0.01;
        let total: f32 = self.grid.get(g..end).map(|x| x.iter().sum()).unwrap_or(0.0);
        // Grid lines where the cell's parts start (after the first, at `g`).
        let mut lines = Vec::with_capacity(n);
        for k in 1..n {
            let x = total * k as f32 / n as f32;
            let mut acc = 0.0;
            let mut hit = None;
            for j in g..end {
                let w = self.grid.get(j).copied().unwrap_or(0.0);
                if j > g && (x - acc).abs() <= EPS {
                    hit = Some((j, None));
                    break;
                }
                if x < acc + w - EPS {
                    if x > acc + EPS {
                        hit = Some((j, Some((x - acc, w - (x - acc)))));
                    }
                    break;
                }
                acc += w;
            }
            match hit {
                // An existing grid line is where the border goes.
                Some((j, None)) => lines.push(j),
                // A new grid line inside column `j`: every cell over `j` widens over both halves.
                Some((j, Some((left, right)))) if self.grid.len() < MAX_COLS => {
                    if let Some(w) = self.grid.get_mut(j) {
                        *w = left;
                    }
                    self.grid.insert(j + 1, right);
                    for rr in 0..self.rows.len() {
                        if let Some(ci) = self.cell_at_grid(rr, j)
                            && let Some(cell) = self.rows.get_mut(rr).and_then(|row| row.cells.get_mut(ci))
                        {
                            cell.props.span = cell.props.span.max(1).saturating_add(1);
                        }
                    }
                    end += 1;
                    lines.push(j + 1);
                }
                _ => {}
            }
        }
        lines.dedup();
        // The cell now spans `g..end`: cut it at the new lines.
        let grid = self.grid.clone();
        let width = |a: usize, b: usize| grid.get(a..b).map(|x| x.iter().sum::<f32>());
        let Some(row) = self.rows.get_mut(r) else { return };
        let mut a = g;
        let mut parts = Vec::with_capacity(lines.len() + 1);
        for &b in lines.iter().chain(std::iter::once(&end)) {
            if b > a {
                parts.push((a, b));
                a = b;
            }
        }
        for (k, &(a, b)) in parts.iter().enumerate() {
            if k == 0 {
                if let Some(cell) = row.cells.get_mut(c) {
                    cell.props.span = (b - a) as u32;
                    cell.props.width = width(a, b);
                }
            } else {
                let mut nc = Cell::empty();
                nc.props.span = (b - a) as u32;
                nc.props.width = width(a, b);
                row.cells.insert((c + k).min(row.cells.len()), nc);
            }
        }
    }

    pub fn total_width(&self) -> f32 {
        self.grid.iter().sum()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A 3×3 table, 100 pt columns: (0,0)–(0,1) merged across, (1,2)–(2,2) merged down.
    fn merged() -> Table {
        let mut t = Table::new(3, 3, 300.0);
        t.merge(0, 0, 0, 1);
        t.merge(1, 2, 2, 2);
        assert_eq!(t.rows[0].cells.iter().map(Cell::span).collect::<Vec<_>>(), [2, 1]);
        assert_eq!((t.rows[1].cells[2].props.vmerge, t.rows[2].cells[2].props.vmerge), (VMerge::Restart, VMerge::Continue));
        t
    }

    fn spans(t: &Table) -> Vec<Vec<usize>> {
        t.rows.iter().map(|r| r.cells.iter().map(Cell::span).collect()).collect()
    }

    /// #1744 review: a cell merged across that is partly selected takes its whole width, and the
    /// other rows widen with it, so the copy isn't ragged.
    #[test]
    fn slice_widens_to_a_merged_cell_in_every_row() {
        let t = merged().slice((0, 1), (1, 2));
        assert_eq!(t.grid, [100.0, 100.0, 100.0]);
        assert_eq!(spans(&t), [vec![2, 1], vec![1, 1, 1]]);
        // Rows without the merged cell: just the selected columns.
        let t = merged().slice((1, 2), (1, 2));
        assert_eq!(t.grid, [100.0, 100.0]);
        assert_eq!(spans(&t), [vec![1, 1], vec![1, 1]]);
    }

    /// #1744 review: a cell continuing a vertical merge from a row left out starts it instead.
    #[test]
    fn slice_starts_a_vertical_merge_whose_first_row_is_left_out() {
        let t = merged().slice((2, 2), (0, 2));
        assert_eq!(t.rows.len(), 1);
        assert_eq!(t.rows[0].cells[2].props.vmerge, VMerge::Restart);
        // Both rows kept: the merge is kept as it is.
        let t = merged().slice((1, 2), (2, 2));
        assert_eq!((t.rows[0].cells[0].props.vmerge, t.rows[1].cells[0].props.vmerge), (VMerge::Restart, VMerge::Continue));
    }

    /// #1744 review: part of the columns is as wide as they are; content controls around rows
    /// and the table aren't copied.
    #[test]
    fn slice_width_and_controls() {
        let mut t = Table::new(3, 3, 300.0);
        (t.props.width, t.props.width_pct) = (Some(300.0), Some(50.0));
        let wrap = ControlWrap { open: vec![crate::control::ContentControl::default()], close: 1 };
        t.controls = wrap.clone();
        t.rows[1].controls = wrap;
        let part = t.slice((0, 2), (2, 2));
        assert_eq!((part.grid.len(), part.props.width, part.props.width_pct), (1, Some(100.0), None));
        let whole = t.slice((0, 2), (0, 2));
        assert_eq!((whole.props.width, whole.props.width_pct), (Some(300.0), Some(50.0)), "all columns keep the table's width");
        let row = t.slice((1, 1), (0, 2));
        assert!(row.controls.is_empty() && row.rows[0].controls.is_empty());
        // Nothing in range: no rows.
        assert!(t.slice((5, 6), (0, 2)).rows.is_empty());
        assert!(t.slice((0, 2), (9, 9)).rows.is_empty());
    }

    #[test]
    fn new_table_shape() {
        let t = Table::new(3, 4, 468.0);
        assert_eq!(t.rows.len(), 3);
        assert_eq!(t.cols(), 4);
        assert_eq!(t.total_width(), 468.0);
        let huge = Table::new(1_000_000, 1000, 100.0);
        assert_eq!(huge.rows.len(), MAX_ROWS);
        assert_eq!(huge.cols(), MAX_COLS);
    }

    #[test]
    fn row_col_ops() {
        let mut t = Table::new(2, 2, 200.0);
        t.insert_row(1, 0);
        assert_eq!(t.rows.len(), 3);
        t.insert_col(1);
        assert_eq!(t.cols(), 3);
        assert!(t.rows.iter().all(|r| r.cells.len() == 3));
        t.delete_col(0);
        assert_eq!(t.cols(), 2);
        t.delete_row(0);
        assert_eq!(t.rows.len(), 2);
        t.delete_row(99);
        t.delete_col(99);
    }

    #[test]
    fn merge_and_split() {
        let mut t = Table::new(2, 3, 300.0);
        t.merge_right(0, 0, 1);
        assert_eq!(t.rows[0].cells.len(), 2);
        assert_eq!(t.rows[0].cells[0].span(), 2);
        assert_eq!(t.cell_at_grid(0, 1), Some(0));
        assert_eq!(t.grid_col(0, 1), 2);
        t.insert_col(1);
        assert_eq!(t.rows[0].cells[0].span(), 3);
        t.split_cell(0, 0, 3);
        assert_eq!(t.rows[0].cells.len(), 4);
        let mut v = Table::new(3, 2, 100.0);
        v.merge(0, 2, 0, 0);
        assert_eq!(v.rows[0].cells[0].props.vmerge, VMerge::Restart);
        assert_eq!(v.rows[2].cells[0].props.vmerge, VMerge::Continue);
    }
}
