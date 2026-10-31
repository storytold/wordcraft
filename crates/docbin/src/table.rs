//! Tables: the table sprms (sgc 5) carried by a row-end (TTP) mark's grpprl, and the
//! assembly of cell-ended paragraphs into [`Table`] blocks ([MS-DOC] §2.6.3, §2.4.3).

use std::sync::Arc;

use wordcraft_doc::Rgb;
use wordcraft_doc::para::Paragraph;
use wordcraft_doc::props::{Align, Border, BorderStyle, Borders, CellProps, HeightRule, RowProps, TableProps, VAlign, VMerge};
use wordcraft_doc::table::{Cell, Row, Table};
use wordcraft_doc::{Block, Blocks, para_block};

use crate::fmt;
use crate::sprm::{self, Prl};

// Paragraph sprms for table membership.
const P_IN_TABLE: u16 = 0x2416;
const P_TTP: u16 = 0x2417;
/// `sprmPItap`: table depth (we only model depth 1; deeper rows fall back to paragraphs).
const P_ITAP: u16 = 0x6649;

// Table sprms ([MS-DOC] §2.6.3).
const T_JC: u16 = 0x5400;
const T_JC_90: u16 = 0x548A;
const T_DXA_LEFT: u16 = 0x9601;
const T_DXA_GAP_HALF: u16 = 0x9602;
const T_TABLE_HEADER: u16 = 0x3404;
const T_TABLE_BORDERS_80: u16 = 0xD605;
const T_ROW_HEIGHT: u16 = 0x9407;
const T_DEF_TABLE: u16 = 0xD608;
const T_DEF_TABLE_SHD_80: u16 = 0xD609;
/// `sprmTDefTableShd` / `Shd2nd` / `Shd3rd`: 10-byte `Shd`s for cells 0-21, 22-43, 44-62.
const T_DEF_TABLE_SHD: u16 = 0xD612;
const T_DEF_TABLE_SHD_2ND: u16 = 0xD616;
const T_DEF_TABLE_SHD_3RD: u16 = 0xD60C;
/// `sprmTDefTableShdRaw` / `Raw2nd` / `Raw3rd`: the same `Shd` arrays over the same cell
/// ranges, which Word 2007+ writes next to the above.
const T_DEF_TABLE_SHD_RAW: u16 = 0xD670;
const T_DEF_TABLE_SHD_RAW_2ND: u16 = 0xD671;
const T_DEF_TABLE_SHD_RAW_3RD: u16 = 0xD672;
const T_CANT_SPLIT_90: u16 = 0x3403;
const T_MERGE: u16 = 0x5624;
const T_VERT_MERGE: u16 = 0xD62B;
const T_VERT_ALIGN: u16 = 0xD62C;
const T_SET_SHD: u16 = 0xD62D;
const T_CANT_SPLIT: u16 = 0x3466;
const T_CELL_WIDTH: u16 = 0xD635;

/// Most grid columns we build from a `sprmTDefTable` (the model's own cap is 63; one more
/// edge is fine but merged rows can sum spans slightly over, so clamp when assembling).
const MAX_COLS: usize = 63;

/// What a paragraph mark says about its table context, from its PAPX.
#[derive(Default)]
pub(crate) struct RowInfo {
    pub(crate) in_table: bool,
    pub(crate) ttp: bool,
    /// The mark itself was 0x07 (a cell end), set by the walker.
    pub(crate) cell_mark: bool,
    /// Table depth from `sprmPItap` (0 or 1 supported; deeper tables degrade to paragraphs).
    pub(crate) depth: u32,
    /// Cell-edge positions (points) from `sprmTDefTable` — [MS-DOC] defines them per row,
    /// so each row keeps its own; ascending in well-formed rows, the first edge usually 0
    /// (the row origin). Rows without a `sprmTDefTable` keep this empty.
    pub(crate) edges: Vec<f32>,
    height: Option<f32>,
    height_rule: HeightRule,
    header: bool,
    cant_split: bool,
    align: Option<Align>,
    indent: Option<f32>,
    borders: Option<Borders>,
    /// Horizontal merges: (itcFirst, itcLim) cell-index ranges.
    merges: Vec<(u8, u8)>,
    /// Per-cell overrides keyed by (pre-merge) cell index.
    vmerge: Vec<(u8, VMerge)>,
    /// Later entries win: the `Shd` arrays override the `Shd80` fallback before them, and an
    /// automatic `Shd` (`None`) clears it.
    shading: Vec<(u8, Option<Rgb>)>,
    widths: Vec<(u8, f32)>,
    width_pcts: Vec<(u8, f32)>,
    valign: Vec<(u8, VAlign)>,
}

/// Read the table context out of a paragraph mark's PAPX grpprl.
impl RowInfo {
    /// The row height from `sprmTDyaRowHeight`, for tests.
    #[cfg(test)]
    pub(crate) fn height(&self) -> Option<f32> {
        self.height
    }

    /// The fill cell `i` ends up with (the last shading entry for it wins), for tests.
    #[cfg(test)]
    pub(crate) fn shading_of(&self, i: u8) -> Option<Rgb> {
        self.shading.iter().rfind(|(k, _)| *k == i).and_then(|(_, s)| *s)
    }

    /// The width cell `i` ends up with, for tests.
    #[cfg(test)]
    pub(crate) fn width_of(&self, i: u8) -> Option<f32> {
        self.widths.iter().rfind(|(k, _)| *k == i).map(|(_, w)| *w)
    }

    /// The width percentage cell `i` ends up with, for tests.
    #[cfg(test)]
    pub(crate) fn width_pct_of(&self, i: u8) -> Option<f32> {
        self.width_pcts.iter().rfind(|(k, _)| *k == i).map(|(_, w)| *w)
    }
}

pub(crate) fn decode(papx: &[u8]) -> RowInfo {
    let mut r = RowInfo::default();
    let mut gap_half: Option<i16> = None;
    let mut dxa_left: Option<i16> = None;
    for prl in sprm::iter(papx) {
        // The membership sprms are paragraph sprms (sgc 1); the rest here are table sprms
        // (sgc 5), so match opcodes directly.
        match prl.op {
            P_IN_TABLE => r.in_table = prl.operand.first().copied().unwrap_or(0) != 0,
            P_TTP => r.ttp = prl.operand.first().copied().unwrap_or(0) != 0,
            P_ITAP => r.depth = i32_of(&prl).unwrap_or(0).max(0) as u32,
            T_DEF_TABLE => r.edges = def_table_edges(prl.operand),
            T_TABLE_HEADER => r.header = prl.operand.first().copied().unwrap_or(0) != 0,
            T_CANT_SPLIT | T_CANT_SPLIT_90 => r.cant_split = prl.operand.first().copied().unwrap_or(0) != 0,
            T_ROW_HEIGHT => {
                if let Some(v) = s16(&prl) {
                    if v < 0 {
                        r.height_rule = HeightRule::Exact;
                        r.height = Some((-(v as f32) / 20.0).clamp(0.0, 1584.0));
                    } else {
                        r.height_rule = HeightRule::AtLeast;
                        r.height = Some((v as f32 / 20.0).clamp(0.0, 1584.0));
                    }
                }
            }
            T_JC | T_JC_90 => {
                r.align = match u16_of(&prl).unwrap_or(0) {
                    1 => Some(Align::Center),
                    2 => Some(Align::Right),
                    _ => Some(Align::Left),
                };
            }
            T_DXA_LEFT => dxa_left = s16(&prl),
            T_DXA_GAP_HALF => gap_half = s16(&prl),
            T_TABLE_BORDERS_80 => r.borders = table_borders(prl.operand),
            T_MERGE => {
                if let [a, b, ..] = prl.operand
                    && b > a
                {
                    r.merges.push((*a, *b));
                }
            }
            T_VERT_MERGE => {
                // cb, itc, flag: 1 = continues the merge from above, 3 = restarts it.
                if let [_, itc, flag, ..] = prl.operand {
                    let m = match flag {
                        3 => VMerge::Restart,
                        1 => VMerge::Continue,
                        _ => VMerge::None,
                    };
                    if m != VMerge::None {
                        r.vmerge.push((*itc, m));
                    }
                }
            }
            T_SET_SHD => {
                if let Some(rgb) = prl.operand.get(3..).and_then(shd_rgb)
                    && let [a, b, ..] = prl.operand.get(1..3).unwrap_or(&[])
                {
                    for c in *a..(*b).min(MAX_COLS as u8) {
                        r.shading.push((c, Some(rgb)));
                    }
                }
            }
            T_DEF_TABLE_SHD_80 => {
                let cells = prl.operand.get(1..).unwrap_or(&[]);
                for (i, b) in cells.as_chunks::<2>().0.iter().take(MAX_COLS).enumerate() {
                    if let Some(rgb) = shd80_rgb(b) {
                        r.shading.push((i as u8, Some(rgb)));
                    }
                }
            }
            T_DEF_TABLE_SHD | T_DEF_TABLE_SHD_RAW | T_DEF_TABLE_SHD_2ND | T_DEF_TABLE_SHD_RAW_2ND | T_DEF_TABLE_SHD_3RD | T_DEF_TABLE_SHD_RAW_3RD => {
                // Each array covers its own cell range (0-21, 22-43, 44-62); extra entries
                // are ignored so an over-long array can't override the next range.
                let (first, lim) = match prl.op {
                    T_DEF_TABLE_SHD_2ND | T_DEF_TABLE_SHD_RAW_2ND => (22, 44),
                    T_DEF_TABLE_SHD_3RD | T_DEF_TABLE_SHD_RAW_3RD => (44, MAX_COLS),
                    _ => (0, 22),
                };
                let cells = prl.operand.get(1..).unwrap_or(&[]);
                let raw = matches!(prl.op, T_DEF_TABLE_SHD_RAW | T_DEF_TABLE_SHD_RAW_2ND | T_DEF_TABLE_SHD_RAW_3RD);
                for (i, b) in cells.as_chunks::<10>().0.iter().take(lim - first).enumerate() {
                    // In the Raw arrays a nil Shd (ipat 0xFFFF) means "as the table style
                    // shades it", so it leaves any earlier fill alone.
                    if raw && b.get(8..10) == Some(&[0xFF, 0xFF]) {
                        continue;
                    }
                    r.shading.push(((first + i) as u8, shd_rgb(b)));
                }
            }
            T_CELL_WIDTH => {
                // Operand: cb (1 byte), itcFirst (1 byte), itcLim (1 byte), ftsWidth (1 byte), wWidth (2 bytes).
                // [MS-DOC] §2.9.290 sprmTCellWidth (0xD635).
                // ftsWidth: 3 = twips (dxa), 2 = percentage (50ths of a percent).
                if let [_, a, b, fts, w0, w1, ..] = prl.operand {
                    if *fts == 3 {
                        let w = (i16::from_le_bytes([*w0, *w1]) as f32 / 20.0).clamp(0.0, 1584.0);
                        for c in *a..(*b).min(MAX_COLS as u8) {
                            r.widths.push((c, w));
                        }
                    } else if *fts == 2 {
                        let pct = (u16::from_le_bytes([*w0, *w1]) as f32 / 50.0).clamp(0.0, 100.0);
                        for c in *a..(*b).min(MAX_COLS as u8) {
                            r.width_pcts.push((c, pct));
                        }
                    }
                }
            }
            T_VERT_ALIGN => {
                if let [_, a, b, va, ..] = prl.operand {
                    let v = match va {
                        1 => VAlign::Center,
                        2 => VAlign::Bottom,
                        _ => VAlign::Top,
                    };
                    for c in *a..*b {
                        r.valign.push((c, v));
                    }
                }
            }
            _ => {}
        }
    }
    // The table indent is the row origin minus half the cell gap.
    if let Some(l) = dxa_left {
        let gap = gap_half.unwrap_or(0) as f32 / 20.0;
        r.indent = Some(((l as f32 / 20.0) - gap).clamp(-1584.0, 1584.0));
    }
    r
}

/// A finished paragraph plus the bookkeeping the block assembly needs.
pub(crate) struct ParaOut {
    pub(crate) para: Paragraph,
    /// CP just past the terminating mark.
    pub(crate) end_cp: u32,
    /// Table context from the mark's PAPX (empty default = not in a table).
    pub(crate) row: RowInfo,
}

/// Group consecutive in-table paragraphs into [`Table`] blocks; the rest pass through.
/// Rows that never close (or tables nested deeper than we model) degrade to paragraphs so
/// no text is lost.
pub(crate) fn assemble(paras: Vec<ParaOut>) -> Blocks {
    let mut out: Blocks = Vec::new();
    let mut rows: Vec<Row> = Vec::new();
    let mut cells: Vec<Cell> = Vec::new();
    let mut cell: Blocks = Vec::new();
    let mut grid: Vec<f32> = Vec::new();
    // Each row's own cell edges, parallel to `rows` (a row without a `sprmTDefTable` has none).
    let mut row_edges: Vec<Vec<f32>> = Vec::new();
    let mut props = TableProps::default();
    let mut open = false;
    let mut depth_seen = 0u32;

    /// Close the open cell: its blocks (an empty paragraph when it has none) go onto the row's cells.
    fn flush_cell(cells: &mut Vec<Cell>, cell: &mut Blocks) {
        let mut blocks = std::mem::take(cell);
        if blocks.is_empty() {
            blocks.push(para_block(Paragraph::new()));
        }
        cells.push(Cell { props: CellProps { span: 1, ..Default::default() }, blocks, controls: Default::default() });
    }
    /// Turn a row's collected cells into a [`Row`]: the per-cell sprm overrides, a width from the
    /// row's cell edges for cells that have none, then the horizontal merges.
    fn build_row(mut cells: Vec<Cell>, info: &RowInfo) -> Row {
        // Per-cell overrides first (sprm cell indices count pre-merge cells), then merges.
        for (i, c) in cells.iter_mut().enumerate() {
            let i = i as u8;
            if let Some((_, m)) = info.vmerge.iter().find(|(k, _)| *k == i) {
                c.props.vmerge = *m;
            }
            if let Some((_, w)) = info.widths.iter().rfind(|(k, _)| *k == i) {
                c.props.width = Some(*w);
            }
            if let Some((_, pct)) = info.width_pcts.iter().rfind(|(k, _)| *k == i) {
                c.props.width_pct = Some(*pct);
            }
            // A cell with no width of its own is as wide as the row's `sprmTDefTable` edges make it,
            // the width Word's own docx export writes on every cell. Without one, a table that asks
            // for no width anywhere is laid out by its content alone and shrinks.
            if c.props.width.is_none() && c.props.width_pct.is_none() {
                c.props.width = edge_width(&info.edges, usize::from(i));
            }
            if let Some((_, s)) = info.shading.iter().rfind(|(k, _)| *k == i) {
                c.props.shading = *s;
            }
            if let Some((_, v)) = info.valign.iter().rfind(|(k, _)| *k == i) {
                c.props.valign = Some(*v);
            }
        }
        // Rightmost merge first: a merge drains only the cells after its first one, so the original
        // indices of the merges to its left stay valid.
        let mut merges: Vec<(u8, u8)> = info.merges.iter().take(MAX_COLS).copied().collect();
        merges.sort_by_key(|m| std::cmp::Reverse(m.0));
        for &(a, b) in &merges {
            let a = a as usize;
            let b = (b as usize).min(cells.len());
            if a < b && b <= cells.len() && b - a > 1 {
                // A merged cell is as wide as the cells it spans together.
                // When one of them has only a percentage width, the row's outer edges give the sum.
                let merged_width =
                    cells.get(a..b).and_then(|s| s.iter().map(|c| c.props.width).sum::<Option<f32>>()).or_else(|| edge_span(&info.edges, a, b));
                let removed: Vec<Cell> = cells.drain(a + 1..b).collect();
                if let Some(first) = cells.get_mut(a) {
                    first.props.span = ((b - a) as u32).min(63);
                    if merged_width.is_some() {
                        first.props.width = merged_width;
                    }
                    for c in removed {
                        first.blocks.extend(c.blocks.into_iter().filter(|x| x.as_para().is_none_or(|p| !p.is_empty())));
                    }
                    if first.blocks.len() > 1 && first.blocks.first().and_then(|x| x.as_para()).is_some_and(Paragraph::is_empty) {
                        first.blocks.remove(0);
                    }
                }
            }
        }
        Row {
            props: RowProps {
                height: info.height,
                height_rule: info.height_rule,
                header: info.header,
                cant_split: info.cant_split,
                ..Default::default()
            },
            cells,
            controls: Default::default(),
        }
    }
    /// Close the open table: the pending cell and row first, then the rows go out as a [`Table`]
    /// block (or as plain paragraphs when the table is nested deeper than the model goes).
    fn flush_table(
        out: &mut Blocks,
        rows: &mut Vec<Row>,
        row_edges: &mut Vec<Vec<f32>>,
        cells: &mut Vec<Cell>,
        cell: &mut Blocks,
        grid: &[f32],
        props: &TableProps,
        nested: bool,
    ) {
        if !cells.is_empty() || !cell.is_empty() {
            flush_cell(cells, cell);
            // A stray trailing row (no TTP of its own): size its cells from the grid.
            let mut stray = build_row(std::mem::take(cells), &RowInfo::default());
            for (i, c) in stray.cells.iter_mut().enumerate() {
                if c.props.width.is_none() && c.props.span == 1 {
                    c.props.width = grid.get(i).copied();
                }
            }
            rows.push(stray);
            row_edges.push(Vec::new());
        }
        if rows.is_empty() {
            return;
        }
        if nested {
            // Unclosed or nested deeper than we model: keep the text, drop the table shape.
            for r in rows.iter() {
                for c in &r.cells {
                    out.extend(c.blocks.iter().cloned());
                }
            }
        } else {
            let aligned = align_rows(rows, row_edges);
            out.push(Arc::new(Block::Table(Table {
                props: props.clone(),
                grid: aligned.unwrap_or_else(|| grid.to_vec()),
                rows: std::mem::take(rows),
                controls: Default::default(),
            })));
        }
        rows.clear();
        row_edges.clear();
    }

    for p in paras {
        let info = p.row;
        let is_table = info.in_table || info.ttp || info.cell_mark;
        if !is_table {
            if open {
                flush_table(&mut out, &mut rows, &mut row_edges, &mut cells, &mut cell, &grid, &props, depth_seen > 1);
                grid.clear();
                // Table props are per-table (first row carrying the sprm wins within the
                // table): reset the accumulator so one table's borders, align, indent and
                // cell margins never leak into the next (the repro's first table wrote
                // Word's default 108-twip gap, and every later table — including the
                // change history with its own 40-twip gap — inherited 5.4 pt margins).
                props = TableProps::default();
                open = false;
                depth_seen = 0;
            }
            out.push(para_block(p.para));
            continue;
        }
        if !open {
            open = true;
        }
        depth_seen = depth_seen.max(info.depth);
        if grid.is_empty() {
            // Column widths from the table's first `sprmTDefTable`.
            grid = info.edges.windows(2).take(MAX_COLS).map(|w| (w[1] - w[0]).max(0.0)).collect();
        }
        if info.borders.is_some() && props.borders.is_none() {
            props.borders = info.borders;
        }
        if props.align.is_none() {
            props.align = info.align;
        }
        if props.indent.is_none() {
            props.indent = info.indent;
        }
        if info.cell_mark {
            // A row-end (TTP) mark terminates the row. Word writes rows as N content
            // cells each ended by their own cell mark, followed by an empty row-end mark
            // that carries the row's TDefTable (the 3GPP change history: 8 content marks
            // + an empty TTP whose TDefTable defines itcMac = 8). When that mark ends no
            // content of its own and the TDefTable already has a column per flushed cell,
            // it is a pure row terminator, not the end of a phantom (N+1)th cell — such a
            // cell has no column (width None) and shifts every real cell's span one grid
            // column over. A TTP mark with content, or one short of the TDefTable's
            // columns, still ends a real cell (rows without a TDefTable keep today's
            // shape: the count is unknowable, so the mark always flushes).
            let row_end_only = info.ttp && p.para.text.is_empty() && cell.is_empty() && !info.edges.is_empty() && cells.len() + 1 >= info.edges.len();
            // ...but a row-end mark with no cells before it still makes a (one-cell) row.
            let row_end_only = row_end_only && !cells.is_empty();
            if !p.para.text.is_empty() {
                cell.push(para_block(p.para));
            }
            if !row_end_only {
                flush_cell(&mut cells, &mut cell);
            }
            if info.ttp {
                let row = build_row(std::mem::take(&mut cells), &info);
                rows.push(row);
                row_edges.push(info.edges.clone());
            }
        } else {
            cell.push(para_block(p.para));
        }
    }
    if open {
        flush_table(&mut out, &mut rows, &mut row_edges, &mut cells, &mut cell, &grid, &props, depth_seen > 1);
    }
    out
}

/// Cell-edge positions (points) from a `sprmTDefTable` operand: cb u16, itcMac u8, then
/// itcMac+1 twip boundaries followed by the TC80s (skipped). Edges are returned as stored:
/// per-row, in operand order; sorting and capping happen when the table grid is assembled.
fn def_table_edges(operand: &[u8]) -> Vec<f32> {
    let itc = operand.get(2).copied().unwrap_or(0) as usize;
    if itc == 0 {
        return Vec::new();
    }
    // itcMac is a u8, so this allocation stays tiny; the loop stops at the operand's end.
    let mut edges = Vec::with_capacity((itc + 1).min(256));
    for i in 0..=itc {
        match operand.get(3 + i * 2..3 + i * 2 + 2) {
            Some(b) => edges.push(i16::from_le_bytes([b[0], b[1]]) as f32 / 20.0),
            None => break,
        }
    }
    edges
}

/// The width between `sprmTDefTable` edges `a` and `b`, when they give a usable one.
fn edge_span(edges: &[f32], a: usize, b: usize) -> Option<f32> {
    let w = edges.get(b)? - edges.get(a)?;
    (w.is_finite() && w > 0.0).then_some(w)
}

/// The width of cell `i` between its `sprmTDefTable` edges, when they give a usable one.
fn edge_width(edges: &[f32], i: usize) -> Option<f32> {
    edge_span(edges, i, i + 1)
}

/// How far (points) two rows' cell edges may differ and still be the same column boundary.
const EDGE_TOL: f32 = 0.5;

/// A row of a .doc table carries its own cell edges, while the model's rows share one grid. The
/// grid is every row's edges together (two wide cells under a row of four narrow ones, or rows
/// whose edges only partly coincide), and each cell spans the grid columns its own edges cover.
/// `None` when the rows agree (the first row's grid then stands), or when a row has no usable edges
/// or its cells can't be mapped onto the grid: the grid and spans then stay as they were.
fn align_rows(rows: &mut [Row], row_edges: &[Vec<f32>]) -> Option<Vec<f32>> {
    if rows.is_empty() || rows.len() != row_edges.len() {
        return None;
    }
    if row_edges.iter().any(|e| e.len() < 2 || e.iter().any(|x| !x.is_finite()) || e.windows(2).any(|w| w[0] >= w[1])) {
        return None;
    }
    // The grid's boundaries: every row's edges, those within `EDGE_TOL` of each other as one.
    let mut all: Vec<f32> = row_edges.iter().flatten().copied().collect();
    all.sort_by(f32::total_cmp);
    let mut bounds: Vec<f32> = Vec::new();
    for e in all {
        if bounds.last().is_none_or(|&l| e - l > EDGE_TOL) {
            bounds.push(e);
        }
    }
    if bounds.len() > MAX_COLS + 1 {
        return None;
    }
    let col_of = |e: f32| bounds.iter().position(|f| (f - e).abs() <= EDGE_TOL);
    let mut cols_of_row: Vec<Vec<usize>> = Vec::with_capacity(row_edges.len());
    for edges in row_edges {
        let cols = edges.iter().map(|&e| col_of(e)).collect::<Option<Vec<usize>>>()?;
        if !cols.windows(2).all(|w| w[0] < w[1]) {
            return None;
        }
        cols_of_row.push(cols);
    }
    if cols_of_row.iter().all(|c| c.len() == bounds.len()) {
        return None;
    }
    // Work out every row's spans before changing any, so a row that can't be mapped leaves all as they were.
    let mut plans: Vec<(Vec<u32>, u32, u32)> = Vec::with_capacity(rows.len());
    for (row, cols) in rows.iter().zip(&cols_of_row) {
        let mut at = 0usize;
        let mut spans = Vec::with_capacity(row.cells.len());
        for c in &row.cells {
            let span = c.props.span.max(1) as usize;
            let (Some(&l), Some(&r)) = (cols.get(at), cols.get(at + span)) else { return None };
            spans.push(((r - l) as u32).min(63));
            at += span;
        }
        let first = cols.first().copied().unwrap_or(0);
        let last = cols.get(at).copied().unwrap_or(first);
        plans.push((spans, first as u32, (bounds.len() - 1).saturating_sub(last) as u32));
    }
    for (row, (spans, before, after)) in rows.iter_mut().zip(plans) {
        for (c, s) in row.cells.iter_mut().zip(spans) {
            c.props.span = s;
        }
        row.props.grid_before = before;
        row.props.grid_after = after;
    }
    Some(bounds.windows(2).map(|w| w[1] - w[0]).collect())
}

/// Table borders from `sprmTTableBorders80`: cb (0x18) then six Brc80s in the order top,
/// left, bottom, right, inside-horizontal, inside-vertical.
fn table_borders(operand: &[u8]) -> Option<Borders> {
    let b = |i: usize| operand.get(1 + i * 4..1 + i * 4 + 4).and_then(brc80);
    Some(Borders { top: b(0), left: b(1), bottom: b(2), right: b(3), between: b(4), inside_v: b(5) })
}

/// One Brc80: width in 1/8 pt, type, palette colour, space/flags byte.
fn brc80(b: &[u8]) -> Option<Border> {
    let [w, ty, ico, flags] = b else { return None };
    let style = match ty {
        0x00 => BorderStyle::None,
        0x02 | 0x15 => BorderStyle::Thick,
        0x03 => BorderStyle::Double,
        0x04 | 0x05 => BorderStyle::Dotted,
        0x06 | 0x07 => BorderStyle::Dashed,
        0x08 | 0x09 => BorderStyle::DotDash,
        0x0A | 0x18 => BorderStyle::Wave,
        0x11 => BorderStyle::Triple,
        _ => BorderStyle::Single,
    };
    if *ty == 0 && *w == 0 {
        return None;
    }
    Some(Border { style, width: (*w as f32 / 8.0).clamp(0.25, 12.0), color: fmt::ico(*ico), space: (flags & 0x1F) as f32 })
}

/// Background colour from a 10-byte `Shd`: cvFore, cvBack (COLORREFs), ipat. Solid patterns
/// show the foreground; clear (0) and the other patterns show the background (Word writes a
/// plain cell fill as clear with cvBack); nil (0xFFFF) and automatic colours are no fill.
fn shd_rgb(b: &[u8]) -> Option<Rgb> {
    let cv = |b: &[u8]| -> Option<Rgb> {
        match b {
            [_, _, _, 0xFF] => None, // fAuto
            [r, g, bl, _] => Some(Rgb(*r, *g, *bl)),
            _ => None,
        }
    };
    let ipat = b.get(8..10).map(|x| u16::from_le_bytes([x[0], x[1]]))?;
    match ipat {
        0xFFFF => None,
        1 => cv(b.get(0..4)?),
        _ => cv(b.get(4..8)?),
    }
}

/// Background colour from a 2-byte `Shd80` (icoFore, icoBack, ipat bit-fields). As in
/// [`shd_rgb`], solid shows the foreground and clear (0) the background, which is how Word
/// 97 writes a plain fill; ico 0 is automatic (no fill).
fn shd80_rgb(b: &[u8]) -> Option<Rgb> {
    let v = u16::from_le_bytes([*b.first()?, *b.get(1)?]);
    let fore = v & 0x1F;
    let back = (v >> 5) & 0x1F;
    let ipat = v >> 10;
    if fore == 0x1F && back == 0x1F {
        return None;
    }
    fmt::ico(if ipat == 1 { fore as u8 } else { back as u8 })
}

fn u16_of(p: &Prl) -> Option<u16> {
    match p.operand {
        [b0, b1, ..] => Some(u16::from_le_bytes([*b0, *b1])),
        _ => None,
    }
}

fn s16(p: &Prl) -> Option<i16> {
    u16_of(p).map(|v| v as i16)
}

fn i32_of(p: &Prl) -> Option<i32> {
    match p.operand {
        [b0, b1, b2, b3, ..] => Some(i32::from_le_bytes([*b0, *b1, *b2, *b3])),
        _ => None,
    }
}
