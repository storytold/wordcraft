//! A table row's properties, carried by its row-end mark ([MS-DOC] §2.4.3, §2.6.3): the cell
//! edges and per-cell borders (`sprmTDefTable`), the table's borders, alignment and indent, row
//! height and flags, and the cells' shading, widths, vertical merges and alignment.

use wordcraft_doc::props::{Align, Border, Borders, HeightRule, VAlign, VMerge};
use wordcraft_doc::table::Table;

use super::grpprl::{Grpprl, brc, brc80, shd, twips};

const T_JC90: u16 = 0x5400;
const T_JC: u16 = 0x548A;
const T_DXA_LEFT: u16 = 0x9601;
const T_DXA_GAP_HALF: u16 = 0x9602;
const T_CANT_SPLIT90: u16 = 0x3403;
const T_TABLE_HEADER: u16 = 0x3404;
const T_CANT_SPLIT: u16 = 0x3466;
const T_TABLE_BORDERS80: u16 = 0xD605;
const T_ROW_HEIGHT: u16 = 0x9407;
const T_DEF_TABLE: u16 = 0xD608;
const T_DEF_TABLE_SHD: [u16; 3] = [0xD612, 0xD616, 0xD60C];
const T_TABLE_BORDERS: u16 = 0xD613;
const T_TABLE_WIDTH: u16 = 0xF614;
const T_VERT_MERGE: u16 = 0xD62B;
const T_VERT_ALIGN: u16 = 0xD62C;
const T_CELL_PADDING_DEFAULT: u16 = 0xD634;
const T_CELL_WIDTH: u16 = 0xD635;

/// Most cells a row holds (`itcMac` MUST be at most 63).
pub(crate) const MAX_CELLS: usize = 63;
/// Word's default space left of a cell's text (5.4 pt), when the table doesn't say.
const DEFAULT_GAP: f32 = 5.4;

/// The TAPX sprms of row `ri` of `t` (of `rows`), for its row-end mark; `style_borders` are
/// the borders of the table's style, for a table without its own.
pub(crate) fn row_end(t: &Table, ri: usize, rows: usize, style_borders: Option<Borders>) -> Grpprl {
    let mut g = Grpprl::default();
    let Some(row) = t.rows.get(ri) else { return g };
    let cells = &row.cells.get(..row.cells.len().min(MAX_CELLS)).unwrap_or(&[]);
    let n = cells.len();
    let tp = &t.props;

    // Alignment, indent and the cell gap.
    let jc: u16 = match tp.align {
        Some(Align::Center) => 1,
        Some(Align::Right) => 2,
        _ => 0,
    };
    if jc != 0 {
        g.u16(T_JC90, jc);
        g.u16(T_JC, jc);
    }
    let gap = tp.cell_margins.map_or(DEFAULT_GAP, |m| m[1]).clamp(0.0, 100.0);
    g.i16(T_DXA_GAP_HALF, twips(gap));
    let indent = tp.indent.unwrap_or(0.0);
    g.i16(T_DXA_LEFT, twips(indent + gap));
    if let Some(m) = tp.cell_margins {
        // CSSAOperand: itcFirst, itcLim, grfbrc (top 1, left 2, bottom 4, right 8), ftsWidth
        // (3 = twips), wWidth.
        for (side, v) in [(1u8, m[0]), (2, m[1]), (4, m[2]), (8, m[3])] {
            let w = twips(v.max(0.0)) as u16;
            let mut op = vec![0, n.max(1) as u8, side, 3];
            op.extend_from_slice(&w.to_le_bytes());
            g.var(T_CELL_PADDING_DEFAULT, &op);
        }
    }
    if let Some(pct) = tp.width_pct {
        // ftsWidth 2: fiftieths of a percent.
        let mut op = vec![2u8];
        op.extend_from_slice(&((pct.clamp(0.0, 600.0) * 50.0).round() as u16).to_le_bytes());
        g.raw(T_TABLE_WIDTH, &op);
    } else if let Some(w) = tp.width {
        let mut op = vec![3u8];
        op.extend_from_slice(&(twips(w.max(0.0)) as u16).to_le_bytes());
        g.raw(T_TABLE_WIDTH, &op);
    }

    // Row height and flags.
    if let Some(h) = row.props.height {
        let tw = twips(h.max(0.0));
        g.i16(T_ROW_HEIGHT, if row.props.height_rule == HeightRule::Exact { tw.saturating_neg() } else { tw });
    }
    if row.props.header {
        g.u8(T_TABLE_HEADER, 1);
    }
    if row.props.cant_split {
        g.u8(T_CANT_SPLIT90, 1);
        g.u8(T_CANT_SPLIT, 1);
    }

    // The table's borders: its own, else its style's.
    let borders = tp.borders.or(style_borders).unwrap_or_default();
    let six = [borders.top, borders.left, borders.bottom, borders.right, borders.between, borders.inside_v];
    let mut b80 = Vec::with_capacity(24);
    let mut b = Vec::with_capacity(48);
    for side in &six {
        b80.extend_from_slice(&side.as_ref().map_or([0; 4], brc80));
        b.extend_from_slice(&side.as_ref().map_or([0; 8], brc));
    }
    g.var(T_TABLE_BORDERS80, &b80);
    g.var(T_TABLE_BORDERS, &b);

    // Cell edges: each cell spans its grid columns (from the row's `grid_before`), else its own
    // preferred width; the edges start at the table's indent.
    let widths = cell_widths(t, ri);
    let mut edges: Vec<i16> = Vec::with_capacity(n + 1);
    let mut x = twips(indent);
    edges.push(x);
    for w in &widths {
        x = x.saturating_add(twips(*w).max(1));
        edges.push(x);
    }
    // TDefTableOperand: cb, itcMac, rgdxaCenter, rgTc80.
    let mut body = vec![n as u8];
    for e in &edges {
        body.extend_from_slice(&e.to_le_bytes());
    }
    for (ci, cell) in cells.iter().enumerate() {
        let mut grf: u16 = 0;
        match cell.props.vmerge {
            VMerge::Restart => grf |= 0x20 | 0x40,
            VMerge::Continue => grf |= 0x20,
            VMerge::None => {}
        }
        grf |= valign_bits(cell.props.valign) << 7;
        grf |= 3 << 9; // ftsWidth: twips
        if cell.props.no_wrap {
            grf |= 1 << 13;
        }
        body.extend_from_slice(&grf.to_le_bytes());
        body.extend_from_slice(&(twips(widths.get(ci).copied().unwrap_or(0.0)) as u16).to_le_bytes());
        let cb = cell_borders(&borders, cell.props.borders.as_ref(), ri, rows, ci, n);
        for side in [cb.top, cb.left, cb.bottom, cb.right] {
            body.extend_from_slice(&side.as_ref().map_or([0; 4], brc80));
        }
    }
    let cb = u16::try_from(body.len() + 1).unwrap_or(u16::MAX);
    let mut op = cb.to_le_bytes().to_vec();
    op.extend_from_slice(&body);
    g.raw(T_DEF_TABLE, &op);

    // Cell shading, in three arrays of up to 22 cells each.
    if cells.iter().any(|c| c.props.shading.is_some()) || tp.shading.is_some() {
        for (k, op) in T_DEF_TABLE_SHD.iter().enumerate() {
            let chunk = cells.get(k * 22..((k + 1) * 22).min(n)).unwrap_or(&[]);
            if chunk.is_empty() {
                break;
            }
            let mut v = Vec::with_capacity(chunk.len() * 10);
            for c in chunk {
                v.extend_from_slice(&shd(c.props.shading.or(tp.shading)));
            }
            g.var(*op, &v);
        }
    }
    // Per-cell preferred widths, vertical merges and alignment, for the readers that take them
    // from sprms rather than from the TC80s.
    for (ci, cell) in cells.iter().enumerate() {
        let i = ci as u8;
        let mut op = vec![i, i + 1];
        if let Some(pct) = cell.props.width_pct {
            op.push(2);
            op.extend_from_slice(&((pct.clamp(0.0, 100.0) * 50.0).round() as u16).to_le_bytes());
        } else {
            op.push(3);
            op.extend_from_slice(&(twips(widths.get(ci).copied().unwrap_or(0.0)) as u16).to_le_bytes());
        }
        g.var(T_CELL_WIDTH, &op);
        match cell.props.vmerge {
            VMerge::Restart => g.var(T_VERT_MERGE, &[i, 3]),
            VMerge::Continue => g.var(T_VERT_MERGE, &[i, 1]),
            VMerge::None => {}
        }
        if let Some(v) = cell.props.valign
            && v != VAlign::Top
        {
            g.var(T_VERT_ALIGN, &[i, i + 1, valign_bits(Some(v)) as u8]);
        }
    }
    g
}

fn valign_bits(v: Option<VAlign>) -> u16 {
    match v {
        Some(VAlign::Center) => 1,
        Some(VAlign::Bottom) => 2,
        _ => 0,
    }
}

/// Each cell's width in points: the grid columns it spans, else its preferred width, else an
/// equal share of a 6-inch row.
fn cell_widths(t: &Table, ri: usize) -> Vec<f32> {
    let Some(row) = t.rows.get(ri) else { return Vec::new() };
    let n = row.cells.len().min(MAX_CELLS);
    let mut col = row.props.grid_before as usize;
    let mut out = Vec::with_capacity(n);
    for cell in row.cells.iter().take(n) {
        let span = (cell.props.span.max(1) as usize).min(MAX_CELLS);
        let from_grid: f32 = t.grid.get(col..col.saturating_add(span).min(t.grid.len())).map_or(0.0, |g| g.iter().sum());
        col = col.saturating_add(span);
        let w = if from_grid > 0.0 && from_grid.is_finite() {
            from_grid
        } else {
            cell.props.width.filter(|w| *w > 0.0 && w.is_finite()).unwrap_or(432.0 / n.max(1) as f32)
        };
        out.push(w.clamp(1.0, 1584.0));
    }
    out
}

/// The borders cell `ci` of `n` in row `ri` of `rows` shows: the table's outer borders on the
/// table's edges, its inside ones between cells, the cell's own over both.
fn cell_borders(table: &Borders, own: Option<&Borders>, ri: usize, rows: usize, ci: usize, n: usize) -> Borders {
    let pick = |own: Option<Border>, edge: bool, outer: Option<Border>, inner: Option<Border>| own.or(if edge { outer } else { inner });
    let own = own.copied().unwrap_or_default();
    Borders {
        top: pick(own.top, ri == 0, table.top, table.between),
        bottom: pick(own.bottom, ri + 1 == rows, table.bottom, table.between),
        left: pick(own.left, ci == 0, table.left, table.inside_v),
        right: pick(own.right, ci + 1 == n, table.right, table.inside_v),
        between: None,
        inside_v: None,
    }
}
