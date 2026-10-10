//! Table layout: column widths, cell boxes, row heights, merged cells, borders and shading,
//! table-style conditional formatting. Rows are laid out as free-standing boxes; pagination
//! moves whole rows or splits them between lines (`split_row`).

use wordcraft_doc::props::{Align, Border, Borders, HeightRule, Rgb, VAlign, VMerge};
use wordcraft_doc::styles::TableStyleProps;
use wordcraft_doc::{StoryRef, Table};
use wordcraft_geom::Rect;

use crate::para::CellText;
use crate::{Ctx, Placed, layout_box};

pub struct RowLayout {
    pub height: f32,
    /// Items relative to the table's top-left (x) and the row's top (y).
    pub items: Vec<Placed>,
}

pub struct TableLayout {
    /// Table x offset within the column.
    pub x: f32,
    pub width: f32,
    /// Right-to-left table: columns run from the right; storage stays logical.
    pub rtl: bool,
    pub rows: Vec<RowLayout>,
}

const DEFAULT_MARGINS: [f32; 4] = [0.0, 5.4, 0.0, 5.4];

/// The table's style with its based-on chain merged.
fn table_style(ctx: &Ctx, t: &Table) -> Option<TableStyleProps> {
    ctx.doc.styles.table_style(t.props.style.as_deref()?)
}

/// The table's default cell margins: its own, else its style's, else Word's.
fn default_margins(t: &Table, style: Option<&TableStyleProps>) -> [f32; 4] {
    t.props.cell_margins.or(style.and_then(|s| s.parts.cell_margins)).unwrap_or(DEFAULT_MARGINS)
}

/// The leading cell's outer margin: how far its text sits inside the table's leading edge —
/// the first cell's left margin, or, for a right-to-left table, the last cell's right margin.
pub(crate) fn leading_cell_margin(ctx: &Ctx, t: &Table) -> f32 {
    let def = default_margins(t, table_style(ctx, t).as_ref());
    if t.props.rtl {
        t.rows.first().and_then(|r| r.cells.last()).and_then(|c| c.props.margins).unwrap_or(def)[3]
    } else {
        first_cell_left_margin_in(t, def)
    }
}

fn first_cell_left_margin_in(t: &Table, def: [f32; 4]) -> f32 {
    t.rows.first().and_then(|r| r.cells.first()).and_then(|c| c.props.margins).unwrap_or(def)[1]
}

pub fn layout_table(ctx: &mut Ctx, story: StoryRef, t: &Table, path: &[u32], avail: f32, depth: usize) -> TableLayout {
    let style = table_style(ctx, t);
    let parts = style.as_ref().map(|s| &s.parts);
    // Cell text formatting per (header row, total row, first column) region, built once per table.
    let mut cell_text: Vec<((bool, bool, bool), CellText)> = Vec::new();
    let ncols = t.cols().max(1);
    // Right-to-left tables show the first logical column on the right; storage stays logical.
    let rtl = t.props.rtl;
    // Column widths.
    let mut grid: Vec<f32> = if t.grid.len() == ncols {
        t.grid.iter().map(|w| if w.is_finite() { w.max(4.0) } else { 72.0 }).collect()
    } else {
        vec![avail / ncols as f32; ncols]
    };
    let mut total: f32 = grid.iter().sum();
    let target = match (t.props.width_pct, t.props.width) {
        (Some(p), _) if p > 0.0 => Some(avail * (p / 100.0).min(1.0)),
        (_, Some(w)) if w > 0.0 => Some(w),
        _ => None,
    };
    if let Some(tw) = target {
        let k = tw / total.max(1.0);
        grid.iter_mut().for_each(|w| *w *= k);
        total = tw;
    }
    if total > avail * 1.5 && !t.props.fixed {
        let k = avail / total.max(1.0);
        grid.iter_mut().for_each(|w| *w *= k);
        total = avail;
    }
    let mut colx = Vec::with_capacity(ncols + 1);
    let mut acc = 0.0;
    for w in &grid {
        colx.push(acc);
        acc += w;
    }
    colx.push(acc);
    // Word 2013 and later (compatibility mode 15) put the table's border at the margin (plus its
    // indent); earlier modes line the first cell's text up with it instead.
    let margins_def = default_margins(t, style.as_ref());
    let indent = t.props.indent.unwrap_or(0.0);
    // The style's borders, overlaid by the table's own side by side.
    let mut tborders = parts.and_then(|p| p.borders).unwrap_or_default();
    if let Some(own) = t.props.borders {
        tborders.overlay(&own);
    }
    let first_cell = t.rows.first().and_then(|r| r.cells.first());
    // The indent side follows the table's direction: a right-to-left table's indent is measured
    // from the text's right edge (`w:tblInd` off the leading margin), with the border half and
    // the leading cell margin taken from the visual-right (logical-last) cell.
    let edge_cell = if rtl { t.rows.first().and_then(|r| r.cells.last()) } else { first_cell };
    let edge_margin = edge_cell.and_then(|c| c.props.margins).unwrap_or(margins_def)[if rtl { 3 } else { 1 }];
    let x = match t.props.align {
        Some(Align::Center) => (avail - total) / 2.0,
        Some(Align::Right) => avail - total,
        _ if ctx.doc.settings.compat_mode >= 15 => {
            // The border is centred on the edge, so half of it sits outside: Word moves the
            // table in by that half.
            let border = edge_cell.and_then(|c| c.props.borders.and_then(|b| if rtl { b.right } else { b.left })).or(if rtl {
                tborders.right
            } else {
                tborders.left
            });
            let inset = indent + border.filter(Border::is_visible).map_or(0.0, |b| b.width.clamp(0.0, 12.0) / 2.0);
            if rtl { avail - total - inset } else { inset }
        }
        _ => {
            if rtl {
                avail - total - indent + edge_margin
            } else {
                indent - edge_margin
            }
        }
    };
    let nrows = t.rows.len();
    let header_rows = t.props.look.header_row;
    // First pass: lay out every cell's content.
    struct CellBox {
        items: Vec<Placed>,
        h: f32,
        x: f32,
        w: f32,
        valign: VAlign,
        fill: Option<Rgb>,
        borders: Borders,
        vmerge: VMerge,
        margins: [f32; 4],
        g: usize,
        span: usize,
    }
    let mut rows: Vec<Vec<CellBox>> = Vec::with_capacity(nrows);
    let mut heights: Vec<f32> = Vec::with_capacity(nrows);
    for (ri, row) in t.rows.iter().enumerate() {
        let mut g = 0usize;
        let mut cells = Vec::with_capacity(row.cells.len());
        let mut rh = 0.0f32;
        // The tallest cell margins plus border bands in the row.
        let mut row_insets = 0.0f32;
        let is_header = header_rows && ri == 0;
        let is_total = t.props.look.total_row && ri + 1 == nrows && nrows > 1;
        let band = t.props.look.banded_rows && !is_header && (ri - usize::from(header_rows)) % 2 == 0;
        for (ci, cell) in row.cells.iter().enumerate() {
            let span = cell.span();
            let lx0 = colx.get(g).copied().unwrap_or(acc);
            let lx1 = colx.get((g + span).min(ncols)).copied().unwrap_or(acc);
            // Mirror logical columns into visual positions; margins stay physical.
            let (x0, x1) = if rtl { (acc - lx1, acc - lx0) } else { (lx0, lx1) };
            let margins = cell.props.margins.unwrap_or(margins_def);
            let cw = (x1 - x0 - margins[1] - margins[3]).max(4.0);
            let mut fill = cell.props.shading;
            let region = (is_header, is_total, t.props.look.first_column && g == 0);
            if let Some(p) = parts {
                if is_header {
                    fill = fill.or(p.header_fill);
                }
                if band && fill.is_none() {
                    fill = p.band_fill;
                }
                fill = fill.or(p.fill);
            }
            if let Some(st) = &style
                && !cell_text.iter().any(|(r, _)| *r == region)
            {
                // The whole table's formatting, then column, then row conditional formatting
                // (later regions win, ECMA-376 §17.7.6).
                let mut chr = st.chr.clone();
                if region.2 {
                    chr.overlay(&st.parts.first_col_chr);
                }
                if is_header {
                    chr.overlay(&st.parts.header_chr);
                } else if is_total {
                    chr.overlay(&st.parts.total_chr);
                }
                cell_text.push((region, CellText { para: st.para.clone(), chr }));
            }
            let text = cell_text.iter().find(|(r, _)| *r == region).map(|(_, c)| c);
            let mut cpath = path.to_vec();
            cpath.push(ri as u32);
            cpath.push(ci as u32);
            // Effective borders: cell > table (outer vs inside).
            let tb = tborders;
            let edge =
                |own: Option<Border>, outer: bool, outer_b: Option<Border>, inner_b: Option<Border>| own.or(if outer { outer_b } else { inner_b });
            let cb = cell.props.borders.unwrap_or_default();
            let mut borders = Borders {
                top: edge(cb.top, ri == 0, tb.top, tb.between),
                bottom: edge(cb.bottom, ri + 1 == nrows, tb.bottom, tb.between),
                left: edge(cb.left, g == 0, tb.left, tb.inside_v),
                right: edge(cb.right, g + span >= ncols, tb.right, tb.inside_v),
                between: None,
                inside_v: None,
            };
            if is_total && let Some(b) = parts.and_then(|p| p.total_border_top) {
                borders.top = Some(b);
            }
            // Word keeps the cell's text clear of its top border (and the last row's of its bottom
            // border): the border's width sits on top of the cell margin.
            let band = |b: Option<Border>| b.filter(Border::is_visible).map_or(0.0, |b| b.width.clamp(0.0, 12.0));
            let (band_t, band_b) = (band(borders.top), if ri + 1 == nrows { band(borders.bottom) } else { 0.0 });
            let (mut items, h) = if cell.props.vmerge == VMerge::Continue {
                (Vec::new(), 0.0)
            } else {
                layout_box(ctx, story, &cell.blocks, &cpath, cw, text, depth, None)
            };
            for it in &mut items {
                it.translate(x0 + margins[1], margins[0] + band_t);
            }
            let insets = margins[0] + margins[2] + band_t + band_b;
            let h = h + insets;
            if cell.props.vmerge != VMerge::Restart {
                rh = rh.max(h);
            }
            row_insets = row_insets.max(insets);
            cells.push(CellBox {
                items,
                h,
                x: x0,
                w: x1 - x0,
                valign: cell.props.valign,
                fill,
                borders,
                vmerge: cell.props.vmerge,
                margins,
                g,
                span,
            });
            g += span;
        }
        // Word adds the cells' top and bottom margins and border bands to an at-least row height:
        // a 30pt row with 5pt margins and 0.5pt borders is 40.5pt tall even when its text needs less.
        let rh = match (row.props.height, row.props.height_rule) {
            (Some(h), HeightRule::Exact) if h > 0.0 => h,
            (Some(h), _) if h > 0.0 => rh.max(h + row_insets),
            _ => rh,
        };
        heights.push(rh.max(4.0));
        rows.push(cells);
    }
    // Vertically merged cells: grow the last row of the merge if the content is taller.
    for ri in 0..rows.len() {
        let restarts: Vec<(usize, f32, usize)> =
            rows.get(ri).map(|r| r.iter().filter(|c| c.vmerge == VMerge::Restart).map(|c| (c.g, c.h, c.span)).collect()).unwrap_or_default();
        for (g, h, _) in restarts {
            let mut end = ri;
            while rows.get(end + 1).is_some_and(|r| r.iter().any(|c| c.g == g && c.vmerge == VMerge::Continue)) {
                end += 1;
            }
            let have: f32 = heights.get(ri..=end).map(|s| s.iter().sum()).unwrap_or(0.0);
            if h > have
                && let Some(last) = heights.get_mut(end)
            {
                *last += h - have;
            }
        }
    }
    // Grid columns holding a vertical-merge continuation, per row.
    let cont: Vec<Vec<usize>> = t
        .rows
        .iter()
        .map(|r| {
            let mut gx = 0;
            r.cells
                .iter()
                .filter_map(|x| {
                    let g = gx;
                    gx += x.span();
                    (x.props.vmerge == VMerge::Continue).then_some(g)
                })
                .collect()
        })
        .collect();
    // Second pass: place cells with their row height, fills and borders.
    let mut out = Vec::with_capacity(rows.len());
    for (ri, cells) in rows.into_iter().enumerate() {
        let rh = heights.get(ri).copied().unwrap_or(0.0);
        let mut items = Vec::new();
        for (ci, c) in cells.into_iter().enumerate() {
            // A restart cell spans several rows: its height is their sum.
            let mut span_h = rh;
            if c.vmerge == VMerge::Restart {
                let mut k = ri + 1;
                while cont.get(k).is_some_and(|v| v.contains(&c.g)) {
                    span_h += heights.get(k).copied().unwrap_or(0.0);
                    k += 1;
                }
            }
            let rect = Rect::new(c.x, 0.0, c.w, span_h);
            if c.vmerge != VMerge::Continue
                && let Some(f) = c.fill
            {
                items.push(Placed::Fill { rect, color: f });
            }
            let dy = match c.valign {
                VAlign::Top => 0.0,
                VAlign::Center => ((span_h - c.h) / 2.0).max(0.0),
                VAlign::Bottom => (span_h - c.h).max(0.0),
            };
            let _ = c.margins;
            for mut it in c.items {
                it.translate(0.0, dy);
                items.push(it);
            }
            let b = c.borders;
            let (x0, x1, y0, y1) = (c.x, c.x + c.w, 0.0, rh);
            let top_vis = c.vmerge != VMerge::Continue;
            if top_vis && let Some(e) = b.top.filter(Border::is_visible) {
                items.push(Placed::Rule { x0, y0, x1, y1: y0, border: e });
            }
            let last_of_merge = c.vmerge == VMerge::None || !cont.get(ri + 1).is_some_and(|v| v.contains(&c.g));
            if last_of_merge && let Some(e) = b.bottom.filter(Border::is_visible) {
                items.push(Placed::Rule { x0, y0: y1, x1, y1, border: e });
            }
            if let Some(e) = b.left.filter(Border::is_visible) {
                items.push(Placed::Rule { x0, y0, x1: x0, y1, border: e });
            }
            if let Some(e) = b.right.filter(Border::is_visible) {
                items.push(Placed::Rule { x0: x1, y0, x1, y1, border: e });
            }
            items.push(Placed::Cell { rect: Rect::new(c.x, 0.0, c.w, rh), table: wordcraft_doc::Path(path.to_vec()), row: ri, cell: ci, story });
        }
        out.push(RowLayout { height: rh, items });
    }
    TableLayout { x, width: total, rtl, rows: out }
}

/// Split a row at `cut` (points below the row's top) so the part above fits on this page.
/// Paragraph lines break between lines; fills, vertical rules and cell areas are cut; the
/// rest moves up to the top of the continuation. `None` when no line fits above the cut.
pub fn split_row(row: &RowLayout, cut: f32) -> Option<(RowLayout, RowLayout)> {
    if cut <= 0.0 || cut >= row.height {
        return None;
    }
    // Where each paragraph splits, and how far up the continuation moves.
    let mut first_moved = f32::MAX;
    let mut kept_any = false;
    for it in &row.items {
        match it {
            Placed::Lines { para, l0, l1, y, .. } => {
                let base = para.lines.get(*l0).map(|l| l.top).unwrap_or(0.0);
                for k in *l0..*l1 {
                    let Some(l) = para.lines.get(k) else { continue };
                    let (top, bottom) = (y + l.top - base, y + l.top - base + l.height);
                    if bottom <= cut + 0.01 {
                        kept_any = true;
                    } else {
                        first_moved = first_moved.min(top);
                        break;
                    }
                }
            }
            Placed::Image { rect, .. } | Placed::Shape { rect, .. } => {
                if rect.bottom() <= cut + 0.01 {
                    kept_any = true;
                } else {
                    first_moved = first_moved.min(rect.y);
                }
            }
            _ => {}
        }
    }
    if !kept_any || first_moved == f32::MAX {
        return None;
    }
    // Keep a little of the cell's top margin on the continuation.
    let shift = (first_moved - 2.0).max(0.0);
    let (mut a, mut b) = (Vec::new(), Vec::new());
    for it in &row.items {
        match it {
            Placed::Lines { story, path, para, l0, l1, x, y } => {
                let base = para.lines.get(*l0).map(|l| l.top).unwrap_or(0.0);
                let mut split = *l1;
                for k in *l0..*l1 {
                    let Some(l) = para.lines.get(k) else { continue };
                    if y + l.top - base + l.height > cut + 0.01 {
                        split = k;
                        break;
                    }
                }
                if split > *l0 {
                    a.push(Placed::Lines { story: *story, path: path.clone(), para: para.clone(), l0: *l0, l1: split, x: *x, y: *y });
                }
                if split < *l1 {
                    let top = para.lines.get(split).map(|l| y + l.top - base).unwrap_or(*y);
                    b.push(Placed::Lines { story: *story, path: path.clone(), para: para.clone(), l0: split, l1: *l1, x: *x, y: top - shift });
                }
            }
            Placed::Image { rect, .. } | Placed::Shape { rect, .. } | Placed::Object { rect, .. } => {
                if rect.bottom() <= cut + 0.01 {
                    a.push(it.clone());
                } else {
                    let mut it = it.clone();
                    it.translate(0.0, -shift);
                    b.push(it);
                }
            }
            Placed::Fill { rect, color } => {
                a.push(Placed::Fill { rect: Rect::new(rect.x, rect.y, rect.w, (cut - rect.y).max(0.0)), color: *color });
                let h = (rect.bottom() - shift - 0.0).max(0.0);
                b.push(Placed::Fill { rect: Rect::new(rect.x, 0.0, rect.w, h), color: *color });
            }
            Placed::Cell { rect, table, row: r, cell, story } => {
                a.push(Placed::Cell { rect: Rect::new(rect.x, rect.y, rect.w, cut), table: table.clone(), row: *r, cell: *cell, story: *story });
                let h = (rect.bottom() - shift).max(0.0);
                b.push(Placed::Cell { rect: Rect::new(rect.x, 0.0, rect.w, h), table: table.clone(), row: *r, cell: *cell, story: *story });
            }
            Placed::Rule { x0, y0, x1, y1, border } => {
                if (y0 - y1).abs() < 0.01 {
                    // Horizontal: the top edge stays, the bottom edge moves; both fragments close.
                    if *y0 <= 0.01 {
                        a.push(it.clone());
                        b.push(Placed::Rule { x0: *x0, y0: 0.0, x1: *x1, y1: 0.0, border: *border });
                    } else if *y0 >= row.height - 0.01 {
                        a.push(Placed::Rule { x0: *x0, y0: cut, x1: *x1, y1: cut, border: *border });
                        b.push(Placed::Rule { x0: *x0, y0: y0 - shift, x1: *x1, y1: y1 - shift, border: *border });
                    } else if *y0 <= cut {
                        a.push(it.clone());
                    } else {
                        b.push(Placed::Rule { x0: *x0, y0: y0 - shift, x1: *x1, y1: y1 - shift, border: *border });
                    }
                } else {
                    let (t, bt) = (y0.min(*y1), y0.max(*y1));
                    if t < cut {
                        a.push(Placed::Rule { x0: *x0, y0: t, x1: *x1, y1: bt.min(cut), border: *border });
                    }
                    if bt > cut {
                        b.push(Placed::Rule { x0: *x0, y0: (t - shift).max(0.0), x1: *x1, y1: bt - shift, border: *border });
                    }
                }
            }
        }
    }
    Some((RowLayout { height: cut, items: a }, RowLayout { height: (row.height - shift).max(4.0), items: b }))
}
