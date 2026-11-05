//! Resizing a table with the mouse, like Word: drag a border between columns (or the table's
//! left or right edge), a row's bottom border, or the small handle just outside the table's
//! bottom-right corner to scale the whole table. With cells selected, a border of theirs moves
//! only in their rows. The geometry comes from the layout (what is on the page, in
//! page points), so the result is what the drag showed even when autofit or a percentage width
//! made the drawn columns differ from the stored grid. The app sends each finished drag here,
//! so every drag is one undo step.

use serde_json::Value;
use wordcraft_doc::props::{Align, HeightRule, VMerge};
use wordcraft_doc::table::MAX_COLS;
use wordcraft_doc::{Cell, Document, Path, StoryRef, Table};
use wordcraft_geom::Rect;
use wordcraft_layout::{DocLayout, Placed};

use super::sel_result;
use crate::{CmdError, CmdResult, CommandSpec, Session, p};

/// How near (points) a border has to be to the pointer, by default.
pub const REACH: f32 = 3.0;
/// The corner handle's side (points); it sits just outside the table's bottom-right corner.
pub const HANDLE: f32 = 6.0;
/// The narrowest column a drag leaves (points).
pub const MIN_COL: f32 = 6.0;
/// The lowest row a drag leaves (points).
pub const MIN_ROW: f32 = 4.0;
/// The largest column width or row height a drag sets (points; Word's 22 inches).
const MAX_LEN: f32 = 1584.0;

pub fn specs() -> Vec<CommandSpec> {
    vec![CommandSpec::new("table.resizeDrag", "Resize Table by Dragging", "Table Layout › Cell Size", resize_drag).params(
        r#"{"page": n, "x": pt, "y": pt, "toX": pt, "toY": pt, "shift"?: bool, "ctrl"?: bool, "reach"?: pt} (a mouse drag in page points, 0-based page, starting on a table border or the resize handle: a column border moves (the column to its right gives or takes the difference, so the table keeps its width; with ctrl the columns right of it share it in proportion; with shift, or on the table's right edge, only the column left of it changes); the table's left edge moves a left-aligned table's indent, the first column making up the difference (a centred or right-aligned table keeps its alignment: only the first column changes); with cells of the table selected, a border of theirs moves only in their rows; a row's bottom border sets that row's height; the handle just outside the bottom-right corner scales every column and the rows on that page. `reach` (default 3) is how near the start has to be.)"#,
    )]
}

/// What a press on the page would grab.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Edge {
    /// The vertical border at grid line `line` (0 = the table's left edge, 1 = between the first
    /// two columns; the last is the table's right edge).
    Column(usize),
    /// The bottom border of row `row`.
    Row(usize),
    /// The resize handle at the bottom-right corner.
    Corner,
}

/// Who makes up the difference when a column border moves: Word's keys while dragging.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Share {
    /// No key: the column on the right, so the table keeps its width.
    #[default]
    Neighbour,
    /// Shift (and a ruler marker): nobody; only the column on the left changes and the ones
    /// right of it move along.
    LeftOnly,
    /// Ctrl: the columns on the right, in proportion to their widths, so the table keeps its
    /// width.
    Proportional,
}

/// Whether Shift is held.
impl From<bool> for Share {
    fn from(shift: bool) -> Share {
        if shift { Share::LeftOnly } else { Share::Neighbour }
    }
}

/// A row of the table as drawn on one page.
#[derive(Clone, Debug, PartialEq)]
pub struct DrawnRow {
    pub row: usize,
    pub top: f32,
    pub bottom: f32,
    /// Grid lines that are cell borders in this row (a merged cell hides the ones it covers).
    pub(crate) edges: Vec<usize>,
}

/// A table border (or the handle) under the pointer, with the table's drawn geometry.
#[derive(Clone, Debug, PartialEq)]
pub struct ResizeTarget {
    pub edge: Edge,
    pub story: StoryRef,
    pub table: Path,
    /// Drawn grid lines, page x, left to right (columns + 1 of them).
    pub cols: Vec<f32>,
    /// Drawn rows on this page, top to bottom.
    pub rows: Vec<DrawnRow>,
    /// Whether the table's last row is on this page (the handle is shown only then).
    pub ends_here: bool,
    /// When cells of the table are selected and the column border is one of theirs
    /// (`limit_to_selection`): their rows, each with the grid lines its cells' borders are on.
    /// The border moves only there; empty, it moves in the whole column.
    pub only: Vec<(usize, Vec<usize>)>,
}

/// What a drag to a point would make of the table.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Plan {
    /// New column widths (points), when the columns change.
    pub widths: Option<Vec<f32>>,
    /// New heights (points) of these rows.
    pub heights: Vec<(usize, f32)>,
    /// How far (points) the table's left edge moves: a left-aligned table's indent (or a
    /// floating table's offset) changes by as much; a centred or right-aligned table keeps its
    /// alignment.
    pub indent_by: Option<f32>,
    /// New cell widths (points, left to right) of the rows whose border moves on its own
    /// (`ResizeTarget::only`).
    pub cells: Vec<(usize, Vec<f32>)>,
}

impl ResizeTarget {
    pub fn left(&self) -> f32 {
        self.cols.first().copied().unwrap_or(0.0)
    }
    pub fn right(&self) -> f32 {
        self.cols.last().copied().unwrap_or(0.0)
    }
    pub fn top(&self) -> f32 {
        self.rows.first().map_or(0.0, |r| r.top)
    }
    pub fn bottom(&self) -> f32 {
        self.rows.last().map_or(0.0, |r| r.bottom)
    }
    /// The drawn column widths.
    pub fn widths(&self) -> Vec<f32> {
        self.cols.windows(2).map(|w| (w[1] - w[0]).max(0.0)).collect()
    }
    /// The resize handle's square (page points).
    pub fn handle(&self) -> Rect {
        Rect::new(self.right() + 1.0, self.bottom() + 1.0, HANDLE, HANDLE)
    }

    /// The new widths and heights a drag that moved the pointer by (`dx`, `dy`) points gives;
    /// `share` says who makes up a column border's move (`true`: Shift is held).
    pub fn plan(&self, dx: f32, dy: f32, share: impl Into<Share>) -> Plan {
        let share = share.into();
        let (dx, dy) =
            (if dx.is_finite() { dx.clamp(-MAX_LEN, MAX_LEN) } else { 0.0 }, if dy.is_finite() { dy.clamp(-MAX_LEN, MAX_LEN) } else { 0.0 });
        let mut w = self.widths();
        match self.edge {
            // The table's left edge: the table moves in or out and the first column makes up the
            // difference, so the other borders stay.
            Edge::Column(0) => {
                let Some(first) = w.first_mut() else { return Plan::default() };
                let d = dx.clamp(*first - MAX_LEN, (*first - MIN_COL).max(*first - MAX_LEN));
                *first -= d;
                Plan { widths: Some(w), indent_by: Some(d), ..Plan::default() }
            }
            // A border of selected cells: it moves in their rows only, as far as all of them let
            // it, each row sharing the move out among its own cells.
            Edge::Column(line) if !self.only.is_empty() => {
                let rows: Vec<(usize, Vec<f32>, usize)> = self
                    .only
                    .iter()
                    .filter_map(|(r, lines)| {
                        let i = lines.iter().position(|g| *g == line)?.checked_sub(1)?;
                        let xs: Vec<f32> = lines.iter().map(|g| self.cols.get(*g).copied()).collect::<Option<_>>()?;
                        Some((*r, xs.windows(2).map(|x| (x[1] - x[0]).max(0.0)).collect(), i))
                    })
                    .collect();
                let d = rows.iter().filter_map(|(_, w, i)| share_out(&mut w.clone(), *i, dx, share)).min_by(|a, b| a.abs().total_cmp(&b.abs()));
                let Some(d) = d else { return Plan::default() };
                let cells = rows
                    .into_iter()
                    .map(|(r, mut w, i)| {
                        share_out(&mut w, i, d, share);
                        (r, w)
                    })
                    .collect();
                Plan { cells, ..Plan::default() }
            }
            Edge::Column(line) => match line.checked_sub(1).and_then(|i| share_out(&mut w, i, dx, share)) {
                Some(_) => Plan { widths: Some(w), ..Plan::default() },
                None => Plan::default(),
            },
            Edge::Row(row) => {
                let Some(r) = self.rows.iter().find(|r| r.row == row) else { return Plan::default() };
                Plan { heights: vec![(row, (r.bottom - r.top + dy).clamp(MIN_ROW, MAX_LEN))], ..Plan::default() }
            }
            Edge::Corner => {
                let (width, height) = (self.right() - self.left(), self.bottom() - self.top());
                let kx = if width > 1.0 { ((width + dx) / width).clamp(0.01, 100.0) } else { 1.0 };
                let ky = if height > 1.0 { ((height + dy) / height).clamp(0.01, 100.0) } else { 1.0 };
                let widths = ((kx - 1.0).abs() > 0.001).then(|| w.iter().map(|x| (x * kx).clamp(MIN_COL, MAX_LEN)).collect());
                let heights = if (ky - 1.0).abs() > 0.001 {
                    self.rows.iter().map(|r| (r.row, ((r.bottom - r.top) * ky).clamp(MIN_ROW, MAX_LEN))).collect()
                } else {
                    Vec::new()
                };
                Plan { widths, heights, ..Plan::default() }
            }
        }
    }

    /// Page x of grid line `line` once `plan` is applied: in row `row`, when that row's border
    /// moves on its own (`Plan::cells`), else in every row.
    pub fn planned_x(&self, plan: &Plan, row: Option<usize>, line: usize) -> f32 {
        let left = self.left() + plan.indent_by.unwrap_or(0.0);
        if let Some(r) = row
            && let Some((_, w)) = plan.cells.iter().find(|(c, _)| *c == r)
            && let Some(k) = self.only.iter().find(|(o, _)| *o == r).and_then(|(_, lines)| lines.iter().position(|g| *g == line))
        {
            return left + w.iter().take(k).sum::<f32>();
        }
        match &plan.widths {
            Some(w) => left + w.iter().take(line).sum::<f32>(),
            None => self.cols.get(line).copied().unwrap_or(left),
        }
    }

    /// When cells of this table are selected whole (one cell too) or the selection runs across
    /// cells of it (Word selects them whole), and the column border grabbed at page y `y` is one
    /// of theirs, in one of their rows, the drag moves it only in those rows (`only`), as Word
    /// does with selected cells' borders. Not the table's left edge, which moves the whole table,
    /// nor a border of a cell merged down into rows outside the selection. Text selected within
    /// one cell isn't a cell selection.
    pub fn limit_to_selection(&mut self, s: &Session, y: f32) {
        self.only.clear();
        let Edge::Column(line) = self.edge else { return };
        if line == 0 {
            return;
        }
        let Some(t) = s.doc.table(self.story, &self.table) else { return };
        let Some(((r0, r1), (lo, hi))) = self.selected_block(s, t) else { return };
        let here = self.rows.iter().find(|r| y >= r.top && y <= r.bottom).map(|r| r.row);
        if line < lo || line > hi || !here.is_some_and(|r| (r0..=r1).contains(&r)) {
            return;
        }
        let mut only = Vec::new();
        for r in r0..=r1 {
            let Some(row) = t.rows.get(r) else { break };
            let mut lines = vec![0usize];
            for cell in &row.cells {
                let g = lines.last().copied().unwrap_or(0);
                let end = g.saturating_add(cell.span());
                // A merged cell that goes on above or below the selection keeps one width.
                let below =
                    || t.cell_at_grid(r + 1, g).and_then(|c| t.rows.get(r + 1)?.cells.get(c)).is_some_and(|c| c.props.vmerge == VMerge::Continue);
                let past = (r == r0 && cell.props.vmerge == VMerge::Continue) || (r == r1 && cell.props.vmerge != VMerge::None && below());
                if past && (g == line || end == line) {
                    return;
                }
                lines.push(end);
            }
            only.push((r, lines));
        }
        self.only = only;
    }

    /// The selected cells of this table, as their rows (first, last) and the grid lines of
    /// their outer borders (left, right): whole cells (`Session::cell_range`), or a selection
    /// running from one of its cells into another. `None` for text within one cell.
    fn selected_block(&self, s: &Session, t: &Table) -> Option<((usize, usize), (usize, usize))> {
        let lines = |cells: &[(usize, usize)]| -> Option<(usize, usize)> {
            let span = |r: usize, c: usize| t.rows.get(r).and_then(|row| row.cells.get(c)).map_or(1, Cell::span);
            let lo = cells.iter().map(|&(r, c)| t.grid_col(r, c)).min()?;
            let hi = cells.iter().map(|&(r, c)| t.grid_col(r, c).saturating_add(span(r, c))).max()?;
            Some((lo, hi))
        };
        if let Some(range) = s.cell_range().filter(|c| c.story == self.story && c.table == self.table) {
            return Some((range.rows, lines(&range.cells(t))?));
        }
        let (a, b) = s.sel.ordered();
        if s.sel.is_collapsed() || a.story != self.story || b.story != self.story {
            return None;
        }
        let ((ra, ca), (rb, cb)) = (cell_in(&self.table, &a.path)?, cell_in(&self.table, &b.path)?);
        if (ra, ca) == (rb, cb) {
            return None;
        }
        Some(((ra.min(rb), ra.max(rb)), lines(&[(ra, ca), (rb, cb)])?))
    }
}

/// Moves the border right of column `i` of `w` (column widths, left to right) by up to `dx`
/// points, `share` saying who makes up the difference, and no column narrower than `MIN_COL`:
/// how far it moved, or `None` when there's no column `i`.
fn share_out(w: &mut [f32], i: usize, dx: f32, share: Share) -> Option<f32> {
    let left = *w.get(i)?;
    let rest: f32 = w.iter().skip(i + 1).sum();
    let narrowest = w.iter().skip(i + 1).copied().fold(f32::INFINITY, f32::min);
    // On the table's right edge (or with no room on the right) only the left column changes.
    let share = match share {
        Share::Neighbour if i + 1 < w.len() => Share::Neighbour,
        Share::Proportional if rest > 0.0 && narrowest > 0.0 => Share::Proportional,
        _ => Share::LeftOnly,
    };
    let least = MIN_COL - left;
    let d = match share {
        Share::Neighbour => dx.clamp(least, (w.get(i + 1).copied().unwrap_or(0.0) - MIN_COL).max(least)),
        // As far as the narrowest column on the right, shrunk in proportion, stays `MIN_COL`.
        Share::Proportional => dx.clamp(least, (rest * (1.0 - (MIN_COL / narrowest).min(1.0))).max(least)),
        Share::LeftOnly => (left + dx).clamp(MIN_COL, MAX_LEN) - left,
    };
    if let Some(x) = w.get_mut(i) {
        *x = left + d;
    }
    match share {
        Share::Neighbour => {
            if let Some(x) = w.get_mut(i + 1) {
                *x -= d;
            }
        }
        Share::Proportional => w.iter_mut().skip(i + 1).for_each(|x| *x *= (rest - d) / rest),
        Share::LeftOnly => {}
    }
    Some(d)
}

/// The (row, cell) of table `tp` that a position at `p` is in.
fn cell_in(tp: &Path, p: &Path) -> Option<(usize, usize)> {
    let n = tp.0.len();
    if p.0.len() < n + 3 || !p.0.starts_with(&tp.0) {
        return None;
    }
    Some((*p.0.get(n)? as usize, *p.0.get(n + 1)? as usize))
}

/// A table cell's area on a page.
pub(crate) struct CellBox {
    pub(crate) rect: Rect,
    pub(crate) table: Path,
    pub(crate) story: StoryRef,
    pub(crate) row: usize,
    pub(crate) cell: usize,
}

pub(crate) fn cell_boxes(l: &DocLayout, page: usize) -> Vec<CellBox> {
    let Some(pg) = l.pages.get(page) else { return Vec::new() };
    pg.items
        .iter()
        .filter_map(|it| match it {
            Placed::Cell { rect, table, row, cell, story, .. } => {
                Some(CellBox { rect: *rect, table: table.clone(), story: *story, row: *row, cell: *cell })
            }
            _ => None,
        })
        .collect()
}

/// The drawn geometry of one table on a page, from its cells' boxes.
fn geometry(t: &Table, boxes: &[&CellBox]) -> Option<(Vec<f32>, Vec<DrawnRow>)> {
    let n = t.cols();
    if n == 0 || n > wordcraft_doc::table::MAX_COLS || boxes.is_empty() {
        return None;
    }
    let mut lines: Vec<Option<f32>> = vec![None; n + 1];
    let mut rows: Vec<DrawnRow> = Vec::new();
    for b in boxes {
        let g = t.grid_col(b.row, b.cell);
        let span = t.rows.get(b.row).and_then(|r| r.cells.get(b.cell)).map_or(1, wordcraft_doc::Cell::span);
        let end = g.saturating_add(span).min(n);
        if let Some(x) = lines.get_mut(g) {
            x.get_or_insert(b.rect.x);
        }
        if let Some(x) = lines.get_mut(end) {
            x.get_or_insert(b.rect.right());
        }
        // A vertically merged cell's box spans all its rows (#640), so a row ends at its
        // shortest box.
        match rows.iter_mut().find(|r| r.row == b.row) {
            Some(r) => {
                r.top = r.top.min(b.rect.y);
                r.bottom = r.bottom.min(b.rect.bottom());
                r.edges.extend([g, end]);
            }
            None => rows.push(DrawnRow { row: b.row, top: b.rect.y, bottom: b.rect.bottom(), edges: vec![g, end] }),
        }
    }
    // A line no cell on this page ends at: between its drawn neighbours, as the grid shares it.
    let (first, last) = (lines.first().copied().flatten()?, lines.last().copied().flatten()?);
    let weight = |i: usize| t.grid.get(i).copied().filter(|w| t.grid.len() == n && w.is_finite() && *w > 0.0).unwrap_or(1.0);
    let mut cols = Vec::with_capacity(n + 1);
    let mut prev = (0usize, first);
    for i in 0..=n {
        let x = match lines.get(i).copied().flatten() {
            Some(x) => x,
            None => {
                let next = (i + 1..=n).find_map(|j| lines.get(j).copied().flatten().map(|x| (j, x))).unwrap_or((n, last));
                let span: f32 = (prev.0..next.0).map(weight).sum();
                let part: f32 = (prev.0..i).map(weight).sum();
                prev.1 + (next.1 - prev.1) * if span > 0.0 { part / span } else { 0.0 }
            }
        };
        let x = x.max(cols.last().copied().unwrap_or(x));
        if lines.get(i).copied().flatten().is_some() {
            prev = (i, x);
        }
        cols.push(x);
    }
    rows.sort_by_key(|r| r.row);
    // A row whose every cell starts a vertical merge has only boxes that span further: it ends
    // where the next row starts.
    let tops: Vec<(usize, f32)> = rows.iter().map(|r| (r.row, r.top)).collect();
    for (r, (next, top)) in rows.iter_mut().zip(tops.into_iter().skip(1)) {
        if next == r.row.saturating_add(1) && top < r.bottom {
            r.bottom = top.max(r.top);
        }
    }
    Some((cols, rows))
}

/// Every table drawn on a page, with its geometry there (`edge` is a placeholder).
pub(crate) fn tables_on(l: &DocLayout, doc: &Document, page: usize) -> Vec<ResizeTarget> {
    let boxes = cell_boxes(l, page);
    let mut tables: Vec<(StoryRef, &Path)> = Vec::new();
    for b in &boxes {
        if !tables.iter().any(|(s, p)| *s == b.story && **p == b.table) {
            tables.push((b.story, &b.table));
        }
    }
    tables
        .into_iter()
        .filter_map(|(story, path)| {
            let t = doc.table(story, path)?;
            let mine: Vec<&CellBox> = boxes.iter().filter(|b| b.story == story && b.table == *path).collect();
            let (cols, rows) = geometry(t, &mine)?;
            let ends_here = rows.last().is_some_and(|r| r.row + 1 == t.rows.len());
            Some(ResizeTarget { edge: Edge::Corner, story, table: path.clone(), cols, rows, ends_here, only: Vec::new() })
        })
        .collect()
}

fn on_handle(t: &ResizeTarget, x: f32, y: f32, reach: f32) -> bool {
    let h = t.handle();
    t.ends_here && x >= h.x - reach / 2.0 && x <= h.right() + reach / 2.0 && y >= h.y - reach / 2.0 && y <= h.bottom() + reach / 2.0
}

/// The innermost table under page point (`x`, `y`) whose resize handle is on this page, or whose
/// handle is under the point: the one whose handle the app shows.
pub fn table_under(l: &DocLayout, doc: &Document, page: usize, x: f32, y: f32, reach: f32) -> Option<ResizeTarget> {
    if !x.is_finite() || !y.is_finite() {
        return None;
    }
    let reach = if reach.is_finite() { reach.clamp(0.5, 24.0) } else { REACH };
    tables_on(l, doc, page)
        .into_iter()
        .filter(|t| t.ends_here && ((x >= t.left() && x <= t.right() && y >= t.top() && y <= t.bottom()) || on_handle(t, x, y, reach)))
        .max_by_key(|t| t.table.0.len())
}

/// The table border or resize handle within `reach` points of page point (`x`, `y`): the
/// nearest, and of nested tables the innermost.
pub fn resize_target(l: &DocLayout, doc: &Document, page: usize, x: f32, y: f32, reach: f32) -> Option<ResizeTarget> {
    if !x.is_finite() || !y.is_finite() {
        return None;
    }
    let reach = if reach.is_finite() { reach.clamp(0.5, 24.0) } else { REACH };
    let mut best: Option<(f32, usize, ResizeTarget)> = None;
    for mut target in tables_on(l, doc, page) {
        let depth = target.table.0.len();
        let mut offer = |d: f32, edge: Edge, target: &mut ResizeTarget| {
            if d <= reach && best.as_ref().is_none_or(|(bd, bdepth, _)| d < *bd - 0.01 || (d <= *bd + 0.01 && depth > *bdepth)) {
                target.edge = edge;
                best = Some((d, depth, target.clone()));
            }
        };
        if on_handle(&target, x, y, reach) {
            offer(0.0, Edge::Corner, &mut target);
        }
        let (left, right, top, bottom) = (target.left(), target.right(), target.top(), target.bottom());
        if x < left - reach || x > right + reach || y < top - reach || y > bottom + reach {
            continue;
        }
        // Column borders: in the row under the pointer, only lines that are cell borders there.
        let row_here = target.rows.iter().find(|r| y >= r.top && y <= r.bottom).cloned();
        if let Some(r) = &row_here {
            for line in 0..target.cols.len() {
                // The table's left edge is grabbed from outside it (a point in, for the border's
                // width): inside starts the first cell.
                if r.edges.contains(&line)
                    && (line > 0 || x <= left + 1.0)
                    && let Some(cx) = target.cols.get(line).copied()
                {
                    offer((x - cx).abs(), Edge::Column(line), &mut target);
                }
            }
        }
        // Row borders: the bottom of each drawn row.
        if x >= left && x <= right {
            for r in target.rows.clone() {
                offer((y - r.bottom).abs(), Edge::Row(r.row), &mut target);
            }
        }
    }
    best.map(|b| b.2)
}

fn resize_drag(s: &mut Session, v: &Value) -> CmdResult {
    let page =
        p::u64(v, "page").map(|n| usize::try_from(n).unwrap_or(usize::MAX)).ok_or_else(|| CmdError::Params("`page` (number) is required".into()))?;
    let (x, y) = (p::req_f32(v, "x")?, p::req_f32(v, "y")?);
    let (to_x, to_y) = (p::req_f32(v, "toX")?, p::req_f32(v, "toY")?);
    // Ctrl wins over Shift, as in Word.
    let share = if p::bool(v, "ctrl").unwrap_or(false) { Share::Proportional } else { Share::from(p::bool(v, "shift").unwrap_or(false)) };
    let reach = p::f32(v, "reach").unwrap_or(REACH);
    let l = s.layout();
    if l.pages.get(page).is_none() {
        return Err(CmdError::Params(format!("no page {page}")));
    }
    let mut target =
        resize_target(&l, &s.doc, page, x, y, reach).ok_or_else(|| CmdError::Failed("there's no table border or resize handle there".into()))?;
    target.limit_to_selection(s, y);
    let plan = target.plan(to_x - x, to_y - y, share);
    if plan.widths.is_none() && plan.heights.is_empty() && plan.cells.is_empty() {
        return sel_result(s);
    }
    let t = s.doc.table_mut(target.story, &target.table)?;
    if let Some(widths) = plan.widths {
        set_widths(t, widths);
    }
    for (r, h) in &plan.heights {
        if let Some(row) = t.rows.get_mut(*r) {
            row.props.height = Some(*h);
            if row.props.height_rule != HeightRule::Exact {
                row.props.height_rule = HeightRule::AtLeast;
            }
        }
    }
    let moved = plan.indent_by.is_some_and(|d| move_left_edge(t, d));
    if !plan.cells.is_empty() {
        regrid(t, &target.cols, &plan.cells)?;
    }
    touch(s, &target)?;
    // Where a moved left edge lands isn't always its indent or offset alone (a floating table's
    // anchor): measure where it came to and put it right once.
    if moved
        && let Some(d) = plan.indent_by
        && let Some(got) = drawn_left(&s.layout(), page, &target)
        && (got - (target.left() + d)).abs() > 0.05
    {
        move_left_edge(s.doc.table_mut(target.story, &target.table)?, target.left() + d - got);
        touch(s, &target)?;
    }
    if plan.heights.is_empty() {
        return sel_result(s);
    }
    // An "at least" height leaves out the cells' margins and border bands, which the layout adds
    // (and which depend on the table style), and an exact one the bottom margin: measure what the
    // rows came to and take the extra off once. A row its text holds taller keeps the dragged
    // height.
    let drawn = drawn_heights(&s.layout(), &s.doc, page, &target);
    let mut fixed = Vec::new();
    let t = s.doc.table_mut(target.story, &target.table)?;
    for (r, want) in &plan.heights {
        let Some(got) = drawn.iter().find(|(i, _)| i == r).map(|(_, h)| *h) else { continue };
        if let Some(row) = t.rows.get_mut(*r)
            && matches!(row.props.height_rule, HeightRule::AtLeast | HeightRule::Exact)
            && got > want + 0.05
        {
            row.props.height = Some((want - (got - want)).max(1.0));
            fixed.push((*r, *want, got));
        }
    }
    if fixed.is_empty() {
        return sel_result(s);
    }
    touch(s, &target)?;
    let again = drawn_heights(&s.layout(), &s.doc, page, &target);
    let t = s.doc.table_mut(target.story, &target.table)?;
    for (r, want, got) in fixed {
        let now = again.iter().find(|(i, _)| *i == r).map_or(got, |(_, h)| *h);
        if now >= got - 0.05
            && let Some(row) = t.rows.get_mut(r)
        {
            row.props.height = Some(want);
        }
    }
    touch(s, &target)?;
    sel_result(s)
}

/// Moves `t`'s left edge `by` points: its indent, or a floating table's offset; whether it did.
/// A centred or right-aligned table keeps its alignment, as in Word: the first column's new width
/// is all that changes, and the table stays centred or on its right edge.
fn move_left_edge(t: &mut Table, by: f32) -> bool {
    match t.props.float.as_mut() {
        Some(f) => {
            f.h_align = None;
            f.x = (f.x + by).clamp(-MAX_LEN, MAX_LEN);
            true
        }
        None if matches!(t.props.align, Some(Align::Center | Align::Right)) => false,
        None => {
            t.props.indent = Some((t.props.indent.unwrap_or(0.0) + by).clamp(-MAX_LEN, MAX_LEN));
            true
        }
    }
}

/// Sets `t`'s grid when some rows' cells change width on their own: `rows` gives their new
/// widths, left to right; the other rows keep their cells' borders where they are drawn (the
/// grid lines at `cols`). The grid gets a line wherever a row has a cell border, and each cell
/// spans the columns between its own.
fn regrid(t: &mut Table, cols: &[f32], rows: &[(usize, Vec<f32>)]) -> Result<(), CmdError> {
    let left = cols.first().copied().unwrap_or(0.0);
    let last = cols.len().saturating_sub(1);
    // Every row's cell borders, measured from the table's left edge.
    let borders: Vec<Vec<f32>> = t
        .rows
        .iter()
        .enumerate()
        .map(|(r, row)| match rows.iter().find(|(i, w)| *i == r && w.len() == row.cells.len()) {
            Some((_, w)) => std::iter::once(0.0)
                .chain(w.iter().scan(0.0, |x, cw| {
                    *x += cw;
                    Some(*x)
                }))
                .collect::<Vec<f32>>(),
            None => std::iter::once(0.0)
                .chain(row.cells.iter().scan(0usize, |g, c| {
                    *g = g.saturating_add(c.span());
                    Some(cols.get((*g).min(last)).copied().unwrap_or(left) - left)
                }))
                .collect::<Vec<f32>>(),
        })
        .collect();
    let mut lines: Vec<f32> = borders.iter().flatten().copied().collect();
    lines.sort_by(f32::total_cmp);
    lines.dedup_by(|a, b| (*a - *b).abs() < 0.01);
    if lines.len() > MAX_COLS + 1 {
        return Err(CmdError::Failed(format!("a table can't have more than {MAX_COLS} columns")));
    }
    let line_at = |x: f32| lines.iter().enumerate().min_by(|(_, a), (_, b)| (*a - x).abs().total_cmp(&(*b - x).abs())).map_or(0, |(i, _)| i);
    let mut spans: Vec<Vec<u32>> = Vec::with_capacity(borders.len());
    for b in &borders {
        let at: Vec<usize> = b.iter().map(|x| line_at(*x)).collect();
        let row: Vec<u32> = at.windows(2).map(|g| g[1].saturating_sub(g[0]) as u32).collect();
        if row.contains(&0) {
            return Err(CmdError::Failed("the cells there are too narrow to move that border".into()));
        }
        spans.push(row);
    }
    t.grid = lines.windows(2).map(|x| x[1] - x[0]).collect();
    for (row, row_spans) in t.rows.iter_mut().zip(spans) {
        for (cell, span) in row.cells.iter_mut().zip(row_spans) {
            cell.props.span = span;
        }
    }
    t.props.width = None;
    t.props.width_pct = None;
    super::table::sync_cell_widths(t);
    Ok(())
}

/// The page x of the target table's left edge on `page`, as drawn.
fn drawn_left(l: &DocLayout, page: usize, target: &ResizeTarget) -> Option<f32> {
    cell_boxes(l, page).iter().filter(|b| b.story == target.story && b.table == target.table).map(|b| b.rect.x).min_by(f32::total_cmp)
}

/// Make `widths` (one per grid column) the table's column widths, as drawn: no preferred table
/// width scales them again.
pub(crate) fn set_widths(t: &mut Table, widths: Vec<f32>) {
    if widths.len() == t.cols() {
        t.grid = widths;
        t.props.width = None;
        t.props.width_pct = None;
        super::table::sync_cell_widths(t);
    }
}

/// The drawn height of each of the target table's rows on `page`.
fn drawn_heights(l: &DocLayout, doc: &Document, page: usize, target: &ResizeTarget) -> Vec<(usize, f32)> {
    tables_on(l, doc, page)
        .into_iter()
        .find(|t| t.story == target.story && t.table == target.table)
        .map(|t| t.rows.iter().map(|r| (r.row, r.bottom - r.top)).collect())
        .unwrap_or_default()
}

/// Mark the table's paragraphs changed, so the layout redoes them.
pub(crate) fn touch(s: &mut Session, target: &ResizeTarget) -> Result<(), CmdError> {
    let story = target.story;
    let paths: Vec<Path> =
        s.doc.para_paths(story).into_iter().filter(|q| q.0.len() > target.table.0.len() && q.0.starts_with(&target.table.0)).collect();
    for q in paths {
        s.doc.para_mut(story, &q)?.touch();
    }
    // A new document revision: the next `layout()` lays the change out instead of reusing the
    // cached pages.
    s.touch();
    Ok(())
}

#[cfg(test)]
mod tests;
