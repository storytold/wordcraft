//! Table layout: column widths, cell boxes, row heights, merged cells, borders and shading,
//! table-style conditional formatting. Rows are laid out as free-standing boxes; pagination
//! moves whole rows or splits them between lines (`split_row`).

use wordcraft_doc::numbering::Counters;
use wordcraft_doc::props::{Align, Border, Borders, HeightRule, Rgb, TextDirection, VAlign, VMerge};
use wordcraft_doc::styles::{TableStyleParts, TableStyleProps};
use wordcraft_doc::{Block, Blocks, Paragraph, StoryRef, Table};
use wordcraft_geom::Rect;

use crate::para::{CellText, ClKind, ParaLayout};
use crate::{BoxLayout, Ctx, Placed, layout_box};

pub struct RowLayout {
    pub height: f32,
    /// Items relative to the table's top-left (x) and the row's top (y).
    pub items: Vec<Placed>,
}

pub struct TableLayout {
    /// Table x offset within the column.
    pub x: f32,
    pub width: f32,
    pub rows: Vec<RowLayout>,
}

const DEFAULT_MARGINS: [f32; 4] = [0.0, 5.4, 0.0, 5.4];

/// The longest line turned text gets when its row height isn't exact (a Letter page's body
/// height): longer text wraps.
const TURNED_MAX: f32 = 648.0;
/// The longest line turned text ever gets (Word's largest page, 22 inches).
const TURNED_LIMIT: f32 = 1584.0;

/// A cell whose text is turned, between the two passes of [`layout_table`].
struct Turned {
    turn: TextDirection,
    /// Line length it was laid out with, and how far its lines reach across the cell.
    len: f32,
    across: f32,
    /// List counters and equation count before the cell, to lay it out again the same way.
    snap: (Counters, u32),
    /// Its formatting region (header row, total row, first column, last column, row band, column
    /// band) and cell index in the row.
    region: Region,
    ci: usize,
    /// Top margin plus top border band.
    top: f32,
}

/// How long the lines laid out in `items` are without the room they didn't use: the length
/// turned text needs so that nothing wraps.
fn natural_length(items: &[Placed]) -> f32 {
    let mut len = 0.0f32;
    for it in items {
        let Placed::Lines { para, l0, l1, x, .. } = it else { continue };
        let indents = para.rp.indent_left.max(0.0) + para.rp.indent_right.max(0.0) + para.rp.indent_first.max(0.0);
        for l in para.lines.get(*l0..*l1).unwrap_or(&[]) {
            let (mut lo, mut hi) = (f32::MAX, f32::MIN);
            for v in &l.vis {
                lo = lo.min(v.x);
                hi = hi.max(v.x + v.w);
            }
            if l.vis.is_empty()
                && let (Some(a), Some(b)) = (l.xs.first(), l.xs.last())
            {
                (lo, hi) = (*a, *b);
            }
            let hy = l.hyphen.map_or(0.0, |h| h.2);
            if hi >= lo {
                len = len.max(x.max(0.0) + hi - lo + hy + indents);
            }
        }
    }
    // A little slack, so laying the text out again at this length doesn't wrap it.
    len + 0.5
}

/// The table's style with its based-on chain merged.
fn table_style(ctx: &Ctx, t: &Table) -> Option<TableStyleProps> {
    ctx.doc.styles.table_style(t.props.style.as_deref()?)
}

/// The table's default cell margins: its own, else its style's, else Word's.
fn default_margins(t: &Table, style: Option<&TableStyleProps>) -> [f32; 4] {
    t.props.cell_margins.or(style.and_then(|s| s.parts.cell_margins)).unwrap_or(DEFAULT_MARGINS)
}

/// The most paragraphs [`measure_table_columns`] lays out: a huge table is measured by its first
/// rows only, so AutoFit stays quick.
const MEASURE_BUDGET: usize = 4000;
/// The line length paragraphs are measured at: wide enough that nothing real wraps.
const MEASURE_WIDE: f32 = 15_840.0;
/// What an empty cell's text needs (about a paragraph mark), points.
const EMPTY_TEXT: f32 = 12.0;

/// What one paragraph needs: the longest run of text without a break opportunity (`min`) and
/// its longest line when nothing wraps (`max`), indents included.
fn measure_para(p: &wordcraft_doc::Paragraph, env: &crate::para::ParaEnv) -> (f32, f32) {
    use crate::para::ClKind;
    let pl = crate::para::layout_para(p, env);
    let rp = &pl.rp;
    let indents = rp.indent_left.max(0.0) + rp.indent_right.max(0.0) + rp.indent_first.max(0.0);
    let (mut word, mut min) = (0.0f32, 0.0f32);
    for c in &pl.clusters {
        match c.kind {
            ClKind::Text | ClKind::Object(_) => word += c.adv.max(0.0),
            ClKind::Marker => {}
            _ => {
                min = min.max(word);
                word = 0.0;
            }
        }
        if c.break_after {
            min = min.max(word);
            word = 0.0;
        }
    }
    min = min.max(word);
    let mut max = 0.0f32;
    for l in &pl.lines {
        let (mut lo, mut hi) = (f32::MAX, f32::MIN);
        for k in l.c0..l.c1 {
            let (Some(c), Some(a), Some(b)) = (pl.clusters.get(k), l.cl_left(k), l.cl_right(k)) else { continue };
            lo = lo.min(a.min(b));
            if !matches!(c.kind, ClKind::Space | ClKind::Marker | ClKind::LineBreak | ClKind::PageBreak | ClKind::ColumnBreak) {
                hi = hi.max(a.max(b));
            }
        }
        if hi > lo {
            max = max.max(hi - lo + l.hyphen.map_or(0.0, |h| h.2));
        }
    }
    let min = if min.is_finite() { min } else { 0.0 };
    let max = if max.is_finite() { max.max(min) } else { min };
    (min + indents, max + indents)
}

/// What each grid column of `t` needs for AutoFit to Contents, as (narrowest, widest) in points
/// with cell margins included: the narrowest fits each cell's longest word (text that can't
/// wrap), the widest fits every paragraph on one line. A cell spanning several columns spreads
/// what its columns lack over them. Paragraphs are laid out with the table style's formatting;
/// after [`MEASURE_BUDGET`] paragraphs the remaining rows are skipped.
pub fn measure_table_columns(doc: &wordcraft_doc::Document, t: &Table) -> Vec<(f32, f32)> {
    use wordcraft_doc::Block;
    let style = t.props.style.as_deref().and_then(|id| doc.styles.table_style(id));
    let margins_def = default_margins(t, style.as_ref());
    let ncols = t.cols().clamp(1, wordcraft_doc::table::MAX_COLS);
    let fields = crate::FieldCtx::default();
    let mut cols = vec![(0.0f32, 0.0f32); ncols];
    // Cells spanning several columns: (first column, span, min, max), applied after the others.
    let mut spanned: Vec<(usize, usize, f32, f32)> = Vec::new();
    let mut budget = MEASURE_BUDGET;
    'rows: for (ri, row) in t.rows.iter().enumerate() {
        let mut g = 0usize;
        for cell in &row.cells {
            let span = cell.span();
            let margins = cell.props.margins.unwrap_or(margins_def);
            let side = margins[1].max(0.0) + margins[3].max(0.0);
            let text = style.as_ref().map(|st| region_text(st, region_of(t, style.as_ref(), ri, g, span, ncols)));
            let env = crate::para::ParaEnv {
                doc,
                width: MEASURE_WIDE,
                label: None,
                fields: &fields,
                show_hidden: false,
                hide_deleted: false,
                table: text.as_ref(),
                proofing: false,
                // Measurement uses a fixed environment (like show_hidden/proofing above).
                numeral: crate::NumeralMode::default(),
                exclusions: &[],
                eq_number: 0,
            };
            let (mut min, mut max) = (EMPTY_TEXT, EMPTY_TEXT);
            if cell.props.vmerge != VMerge::Continue && !cell.props.text_direction.is_turned() {
                for b in &cell.blocks {
                    match &**b {
                        Block::Para(p) => {
                            if budget == 0 {
                                break 'rows;
                            }
                            budget -= 1;
                            let (a, b) = measure_para(p, &env);
                            min = min.max(a);
                            max = max.max(b);
                        }
                        // A nested table keeps its own column widths.
                        Block::Table(inner) => {
                            let w: f32 = inner.grid.iter().filter(|w| w.is_finite()).map(|w| w.clamp(0.0, 1584.0)).sum();
                            min = min.max(w);
                            max = max.max(w);
                        }
                    }
                }
            }
            let (min, max) = ((min + side).min(1584.0), (max + side).min(1584.0));
            if span == 1 {
                if let Some(c) = cols.get_mut(g) {
                    c.0 = c.0.max(min);
                    c.1 = c.1.max(max);
                }
            } else {
                spanned.push((g, span, min, max));
            }
            g += span;
        }
    }
    for (g, span, min, max) in spanned {
        let Some(cs) = cols.get_mut(g..(g + span).min(ncols)) else { continue };
        if cs.is_empty() {
            continue;
        }
        let n = cs.len() as f32;
        let (have_min, have_max) = cs.iter().fold((0.0, 0.0), |(a, b), c| (a + c.0, b + c.1));
        for c in cs.iter_mut() {
            c.0 += ((min - have_min) / n).max(0.0);
            c.1 += ((max - have_max) / n).max(0.0);
            c.1 = c.1.max(c.0);
        }
    }
    // A column no cell starts in still needs room for its margins.
    let floor = margins_def[1].max(0.0) + margins_def[3].max(0.0) + EMPTY_TEXT;
    cols.iter_mut().for_each(|c| {
        c.0 = c.0.max(floor);
        c.1 = c.1.max(c.0);
    });
    cols
}

/// Column widths for AutoFit to Contents, the way Word shares the room: every column gets its
/// widest content when that all fits in `avail`; otherwise each gets its narrowest plus a share
/// of the remaining room in proportion to how much wider its content would like to be. When
/// even the narrowest widths don't fit, the columns keep them (the table runs past the margin,
/// as in Word).
pub fn autofit_widths(cols: &[(f32, f32)], avail: f32) -> Vec<f32> {
    let clean = |v: f32| if v.is_finite() { v.clamp(0.0, 1584.0) } else { 0.0 };
    let cols: Vec<(f32, f32)> = cols.iter().map(|&(a, b)| (clean(a), clean(b).max(clean(a)))).collect();
    let avail = clean(avail);
    let (sum_min, sum_max) = cols.iter().fold((0.0f32, 0.0f32), |(a, b), c| (a + c.0, b + c.1));
    if sum_max <= avail {
        return cols.iter().map(|c| c.1).collect();
    }
    if sum_min >= avail || sum_max <= sum_min {
        return cols.iter().map(|c| c.0).collect();
    }
    let k = (avail - sum_min) / (sum_max - sum_min);
    cols.iter().map(|c| c.0 + (c.1 - c.0) * k).collect()
}

/// The first cell's left margin: how far its text sits inside the table's edge.
pub(crate) fn first_cell_left_margin(ctx: &Ctx, t: &Table) -> f32 {
    first_cell_left_margin_in(t, default_margins(t, table_style(ctx, t).as_ref()))
}

fn first_cell_left_margin_in(t: &Table, def: [f32; 4]) -> f32 {
    t.rows.first().and_then(|r| r.cells.first()).and_then(|c| c.props.margins).unwrap_or(def)[1]
}

/// A cell's borders from the table style's conditional regions it is in (`regions`: header row,
/// total row, first column, last column, row band, column band), later regions winning. Row
/// regions give their left/right edges at the table's sides and `inside_v` between cells; column
/// regions their top/bottom edges at the table's top and bottom and `between` between rows.
fn region_borders(p: &TableStyleParts, (ri, nrows): (usize, usize), (first_g, last_g): (bool, bool), regions: [bool; 6]) -> Borders {
    let [header, total, first_col, last_col, band, col_band] = regions;
    let row = |b: Option<Borders>| {
        b.map(|b| Borders {
            top: b.top,
            bottom: b.bottom,
            left: if first_g { b.left } else { b.inside_v },
            right: if last_g { b.right } else { b.inside_v },
            between: None,
            inside_v: None,
        })
    };
    let col = |b: Option<Borders>| {
        b.map(|b| Borders {
            top: if ri == 0 { b.top } else { b.between },
            bottom: if ri + 1 >= nrows { b.bottom } else { b.between },
            left: b.left,
            right: b.right,
            between: None,
            inside_v: None,
        })
    };
    let mut out = Borders::default();
    for (on, b) in [
        (col_band, col(p.col_band_borders)),
        (band, row(p.band_borders)),
        (first_col, col(p.first_col_borders)),
        (last_col, col(p.last_col_borders)),
        (header, row(p.header_borders)),
        (total, row(p.total_borders)),
    ] {
        if on && let Some(b) = b {
            out.overlay(&b);
        }
    }
    out
}

pub fn layout_table(ctx: &mut Ctx, story: StoryRef, t: &Table, path: &[u32], avail: f32, depth: usize) -> TableLayout {
    let style = table_style(ctx, t);
    let parts = style.as_ref().map(|s| &s.parts);
    // Cell text formatting per region, built once per table.
    let mut cell_text: Vec<(Region, CellText)> = Vec::new();
    let ncols = t.cols().max(1);
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
    if !t.props.fixed {
        // Measuring lays paragraphs out once more; it mustn't count their equations twice.
        let eq_count = ctx.eq_count;
        let content = column_content(ctx, t, style.as_ref(), depth);
        ctx.eq_count = eq_count;
        if let Some(content) = content {
            autofit(&mut grid, &content, t, avail);
            // Never wider than the room it had, however long an unbreakable word.
            let fitted: f32 = grid.iter().sum();
            let cap = avail.max(total);
            if fitted > cap {
                grid.iter_mut().for_each(|w| *w *= cap / fitted);
            }
            total = grid.iter().sum();
        }
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
    let x = match t.props.align {
        Some(Align::Center) => (avail - total) / 2.0,
        Some(Align::Right) => avail - total,
        _ if ctx.doc.settings.compat_mode >= 15 => {
            // The border is centred on the edge, so half of it sits outside: Word moves the
            // table in by that half.
            let border = first_cell.and_then(|c| c.props.borders.and_then(|b| b.left)).or(tborders.left);
            indent + border.filter(Border::is_visible).map_or(0.0, |b| b.width.clamp(0.0, 12.0) / 2.0)
        }
        _ => indent - first_cell_left_margin_in(t, margins_def),
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
        /// Turned text: `items` are still in the turned frame (placed in the second pass).
        turned: Option<Turned>,
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
        let band = banded(t, style.as_ref(), ri);
        for (ci, cell) in row.cells.iter().enumerate() {
            let span = cell.span();
            let x0 = colx.get(g).copied().unwrap_or(acc);
            let x1 = colx.get((g + span).min(ncols)).copied().unwrap_or(acc);
            let margins = cell.props.margins.unwrap_or(margins_def);
            let cw = (x1 - x0 - margins[1] - margins[3]).max(4.0);
            let mut fill = cell.props.shading;
            let region = region_of(t, style.as_ref(), ri, g, span, ncols);
            let (_, _, first_col, last_col, _, col_band) = region;
            // Conditional formatting, later regions winning (ECMA-376 §17.7.6): whole table,
            // column bands, row bands, first/last column, header/total row.
            if let Some(p) = parts {
                let row_fill = if is_header {
                    p.header_fill
                } else if is_total {
                    p.total_fill
                } else {
                    None
                };
                let col_fill = if first_col {
                    p.first_col_fill
                } else if last_col {
                    p.last_col_fill
                } else {
                    None
                };
                fill = fill
                    .or(row_fill)
                    .or(col_fill)
                    .or(if band { p.band_fill } else { None })
                    .or(if col_band { p.col_band_fill } else { None })
                    .or(p.fill);
            }
            if let Some(st) = &style
                && !cell_text.iter().any(|(r, _)| *r == region)
            {
                cell_text.push((region, region_text(st, region)));
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
            let (first_g, last_g) = (g == 0, g + span >= ncols);
            let rb = parts
                .map(|p| region_borders(p, (ri, nrows), (first_g, last_g), [is_header, is_total, first_col, last_col, band, col_band]))
                .unwrap_or_default();
            let mut borders = Borders {
                top: edge(cb.top.or(rb.top), ri == 0, tb.top, tb.between),
                bottom: edge(cb.bottom.or(rb.bottom), ri + 1 == nrows, tb.bottom, tb.between),
                left: edge(cb.left.or(if first_g { rb.left } else { rb.inside_v }), first_g, tb.left, tb.inside_v),
                right: edge(cb.right.or(if last_g { rb.right } else { rb.inside_v }), last_g, tb.right, tb.inside_v),
                between: None,
                inside_v: None,
            };
            if is_total
                && parts.is_none_or(|p| p.total_borders.is_none_or(|b| b.top.is_none()))
                && let Some(b) = parts.and_then(|p| p.total_border_top)
            {
                borders.top = Some(b);
            }
            // Word keeps the cell's text clear of its top border (and the last row's of its bottom
            // border): the border's width sits on top of the cell margin.
            let band = |b: Option<Border>| b.filter(Border::is_visible).map_or(0.0, |b| b.width.clamp(0.0, 12.0));
            let (band_t, band_b) = (band(borders.top), if ri + 1 == nrows { band(borders.bottom) } else { 0.0 });
            let insets = margins[0] + margins[2] + band_t + band_b;
            let turn = cell.props.text_direction;
            let mut turned = None;
            let (items, h) = if cell.props.vmerge == VMerge::Continue {
                (Vec::new(), 0.0)
            } else if turn.is_turned() {
                // Turned text: its lines run along the cell's height. An exact row height sets
                // their length; otherwise they get the length the text needs unwrapped and the row
                // grows to fit (the second pass lays them out again if the row ends up taller).
                let snap = (ctx.counters.clone(), ctx.eq_count);
                let len = match (row.props.height, row.props.height_rule) {
                    (Some(h), HeightRule::Exact) if h > 0.0 => (h - insets).clamp(4.0, TURNED_LIMIT),
                    _ => {
                        let probe = layout_box(ctx, story, &cell.blocks, &cpath, TURNED_MAX, text, depth, None).items;
                        (ctx.counters, ctx.eq_count) = (snap.0.clone(), snap.1);
                        natural_length(&probe).clamp(4.0, TURNED_MAX)
                    }
                };
                let BoxLayout { items, height, floats_bottom } = layout_box(ctx, story, &cell.blocks, &cpath, len, text, depth, None);
                let across = height.max(floats_bottom);
                turned = Some(Turned { turn, len, across, snap, region, ci, top: margins[0] + band_t });
                (items, len)
            } else {
                let BoxLayout { mut items, height, floats_bottom } = layout_box(ctx, story, &cell.blocks, &cpath, cw, text, depth, None);
                for it in &mut items {
                    it.translate(x0 + margins[1], margins[0] + band_t);
                }
                // The cell grows to hold its floating tables.
                (items, height.max(floats_bottom))
            };
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
                turned,
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
            let mut dy = match c.valign {
                VAlign::Top => 0.0,
                VAlign::Center => ((span_h - c.h) / 2.0).max(0.0),
                VAlign::Bottom => (span_h - c.h).max(0.0),
            };
            let mut cell_items = c.items;
            if let Some(tn) = c.turned {
                dy = 0.0;
                let m = c.margins;
                // The lines run the cell's whole height (laid out again if the row grew).
                let len = (span_h - (c.h - tn.len)).clamp(4.0, TURNED_LIMIT);
                let mut across = tn.across;
                if (len - tn.len).abs() > 0.01
                    && let Some(cell) = t.rows.get(ri).and_then(|r| r.cells.get(tn.ci))
                {
                    let mut cpath = path.to_vec();
                    cpath.extend([ri as u32, tn.ci as u32]);
                    let text = cell_text.iter().find(|(r, _)| *r == tn.region).map(|(_, c)| c);
                    // Same list numbers as the first layout: lay it out from the same counters.
                    let now = (std::mem::replace(&mut ctx.counters, tn.snap.0), std::mem::replace(&mut ctx.eq_count, tn.snap.1));
                    let b = layout_box(ctx, story, &cell.blocks, &cpath, len, text, depth, None);
                    (cell_items, across) = (b.items, b.height.max(b.floats_bottom));
                    (ctx.counters, ctx.eq_count) = now;
                }
                // Lines stack across the cell from its start edge (right for top-to-bottom text,
                // left for bottom-to-top); the vertical alignment moves them across it.
                let cw = (c.w - m[1] - m[3]).max(0.0);
                let off = match c.valign {
                    VAlign::Top => 0.0,
                    VAlign::Center => ((cw - across) / 2.0).max(0.0),
                    VAlign::Bottom => (cw - across).max(0.0),
                };
                let (ox, oy) = match tn.turn {
                    TextDirection::Up => (c.x + m[1] + off, tn.top + len),
                    _ => (c.x + m[1] + cw - off, tn.top),
                };
                for it in &mut cell_items {
                    it.turn(tn.turn, ox, oy);
                }
            }
            for mut it in cell_items {
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
    TableLayout { x, width: total, rows: out }
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
        // Turned text moves as a whole, like a picture.
        if let Some(r) = it.turned_bounds() {
            if r.bottom() <= cut + 0.01 {
                kept_any = true;
            } else {
                first_moved = first_moved.min(r.y);
            }
            continue;
        }
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
            Placed::Image { rect, .. } | Placed::Shape { rect, .. } | Placed::Graphic { rect, .. } => {
                if rect.bottom() <= cut + 0.01 {
                    kept_any = true;
                } else {
                    first_moved = first_moved.min(rect.y);
                }
            }
            _ => {}
        }
    }
    if !kept_any {
        return None;
    }
    // Every line fits and only a nested table's cell runs past the cut (a floating table's
    // fixed-height row, say): split there rather than move the whole row on. A row that is just
    // taller than its text moves on whole, as in Word.
    let own_depth = row.items.iter().filter_map(|it| if let Placed::Cell { table, .. } = it { Some(table.0.len()) } else { None }).min();
    let nested_past_cut =
        row.items.iter().any(|it| matches!(it, Placed::Cell { rect, table, .. } if Some(table.0.len()) > own_depth && rect.bottom() > cut + 0.01));
    if first_moved == f32::MAX && nested_past_cut {
        first_moved = cut;
    }
    if first_moved == f32::MAX {
        return None;
    }
    // Keep a little of the cell's top margin on the continuation.
    let shift = (first_moved - 2.0).max(0.0);
    let (mut a, mut b) = (Vec::new(), Vec::new());
    for it in &row.items {
        if let Some(r) = it.turned_bounds() {
            let mut it = it.clone();
            if r.bottom() <= cut + 0.01 {
                a.push(it);
            } else {
                it.translate(0.0, -shift);
                b.push(it);
            }
            continue;
        }
        match it {
            Placed::Lines { story, path, para, l0, l1, x, y, turn } => {
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
                    a.push(Placed::Lines { story: *story, path: path.clone(), para: para.clone(), l0: *l0, l1: split, x: *x, y: *y, turn: *turn });
                }
                if split < *l1 {
                    let top = para.lines.get(split).map(|l| y + l.top - base).unwrap_or(*y);
                    b.push(Placed::Lines {
                        story: *story,
                        path: path.clone(),
                        para: para.clone(),
                        l0: split,
                        l1: *l1,
                        x: *x,
                        y: top - shift,
                        turn: *turn,
                    });
                }
            }
            Placed::Image { rect, .. } | Placed::Shape { rect, .. } | Placed::Graphic { rect, .. } | Placed::Object { rect, .. } => {
                if rect.bottom() <= cut + 0.01 {
                    a.push(it.clone());
                } else {
                    let mut it = it.clone();
                    it.translate(0.0, -shift);
                    b.push(it);
                }
            }
            // Shading and cell areas: the part above the cut stays, the rest moves up with the
            // continuation (a nested table's cell below the cut moves whole).
            Placed::Fill { rect, color } => {
                let (above, below) = split_rect(*rect, cut, shift);
                a.extend(above.map(|rect| Placed::Fill { rect, color: *color }));
                b.extend(below.map(|rect| Placed::Fill { rect, color: *color }));
            }
            Placed::Cell { rect, table, row: r, cell, story } => {
                let (above, below) = split_rect(*rect, cut, shift);
                let cell_at = |rect| Placed::Cell { rect, table: table.clone(), row: *r, cell: *cell, story: *story };
                a.extend(above.map(cell_at));
                b.extend(below.map(cell_at));
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

/// The parts of `r` above `cut` and below it, the latter moved up by `shift` (onto the
/// continuation of a row split at `cut`).
fn split_rect(r: Rect, cut: f32, shift: f32) -> (Option<Rect>, Option<Rect>) {
    let above = (r.y < cut).then(|| Rect::new(r.x, r.y, r.w, (r.bottom().min(cut) - r.y).max(0.0)));
    let top = (r.y - shift).max(0.0);
    let below = (r.bottom() > cut).then(|| Rect::new(r.x, top, r.w, (r.bottom() - shift - top).max(0.0)));
    (above, below)
}

/// A cell's region for table-style conditional formatting: (header row, total row, first column,
/// last column, banded row, banded column).
type Region = (bool, bool, bool, bool, bool, bool);

/// The region of the cell at grid column `g` spanning `span` of `ncols` columns in row `ri`.
fn region_of(t: &Table, style: Option<&TableStyleProps>, ri: usize, g: usize, span: usize, ncols: usize) -> Region {
    let (look, nrows) = (&t.props.look, t.rows.len());
    let first_col = look.first_column && g == 0;
    let last_col = look.last_column && g.saturating_add(span) >= ncols && ncols > 1;
    let col_band = look.banded_columns && !first_col && g.saturating_sub(usize::from(look.first_column)).is_multiple_of(2);
    (look.header_row && ri == 0, look.total_row && ri + 1 == nrows && nrows > 1, first_col, last_col, banded(t, style, ri), col_band)
}

/// Whether row `ri` is in an odd band of the table style's banded rows (bands of the style's
/// band size, counted below the header row).
fn banded(t: &Table, style: Option<&TableStyleProps>, ri: usize) -> bool {
    let look = &t.props.look;
    let size = style.and_then(|s| s.parts.band_size).unwrap_or(1).clamp(1, 1000) as usize;
    look.banded_rows && !(look.header_row && ri == 0) && (ri.saturating_sub(usize::from(look.header_row)) / size).is_multiple_of(2)
}

/// The table style's text formatting in `region`: the whole table's, then the column band's, the
/// row band's, the first/last column's and the header/total row's (later regions win,
/// ECMA-376 §17.7.6).
fn region_text(st: &TableStyleProps, (header, total, first_col, last_col, band, col_band): Region) -> CellText {
    let mut chr = st.chr.clone();
    if col_band {
        chr.overlay(&st.parts.col_band_chr);
    }
    if band {
        chr.overlay(&st.parts.band_chr);
    }
    if first_col {
        chr.overlay(&st.parts.first_col_chr);
    }
    if last_col {
        chr.overlay(&st.parts.last_col_chr);
    }
    if header {
        chr.overlay(&st.parts.header_chr);
    } else if total {
        chr.overlay(&st.parts.total_chr);
    }
    CellText { para: st.para.clone(), chr }
}

/// Each column's narrowest and widest content, cell margins included: its longest word and its
/// longest line. A cell spanning columns shares what they lack between them; nested tables count
/// with their own. `None` when every cell is empty.
fn column_content(ctx: &mut Ctx, t: &Table, style: Option<&TableStyleProps>, depth: usize) -> Option<Vec<(f32, f32)>> {
    let margins_def = default_margins(t, style);
    let mut cols = vec![(0.0f32, 0.0f32); t.cols().max(1)];
    let mut spanning = Vec::new();
    let mut any = false;
    for (ri, row) in t.rows.iter().enumerate() {
        let mut g = 0usize;
        for cell in &row.cells {
            let span = cell.span();
            if cell.props.vmerge != VMerge::Continue {
                let text = style.map(|st| region_text(st, region_of(t, style, ri, g, cell.span(), cols.len())));
                let (lo, hi) = blocks_content(ctx, &cell.blocks, text.as_ref(), depth);
                any |= hi > 0.0;
                let m = cell.props.margins.unwrap_or(margins_def);
                let need = (lo + m[1] + m[3], hi + m[1] + m[3]);
                match cols.get_mut(g) {
                    Some(col) if span == 1 => *col = (col.0.max(need.0), col.1.max(need.1)),
                    _ => spanning.push((g, span, need)),
                }
            }
            g += span;
        }
    }
    for (g, span, need) in spanning {
        let Some(cs) = cols.get_mut(g..(g + span).min(t.cols())) else { continue };
        let n = cs.len().max(1) as f32;
        let (have_lo, have_hi) = cs.iter().fold((0.0, 0.0), |(l, h), c| (l + c.0, h + c.1));
        for c in cs {
            c.0 += (need.0 - have_lo).max(0.0) / n;
            c.1 += (need.1 - have_hi).max(0.0) / n;
        }
    }
    any.then_some(cols)
}

/// The narrowest and widest a block list can be laid out.
fn blocks_content(ctx: &mut Ctx, blocks: &Blocks, text: Option<&CellText>, depth: usize) -> (f32, f32) {
    let (mut lo, mut hi) = (0.0f32, 0.0f32);
    for b in blocks.iter() {
        let (l, h) = match &**b {
            Block::Para(p) => {
                let label = list_level(ctx, p).map(|l| (String::new(), l));
                para_content(&ctx.para_labelled(p, UNBOUNDED, text, &[], label))
            }
            // A nested table: its fixed width, else its columns' but at least its preferred width
            // (its own, else its grid's when it has preferred widths at all).
            Block::Table(nt) if depth < 8 => {
                let grid = if is_automatic(nt) { 0.0 } else { nt.grid.iter().filter(|w| w.is_finite()).map(|w| w.max(0.0)).sum() };
                let width = nt.props.width.filter(|w| w.is_finite()).unwrap_or(grid).clamp(0.0, UNBOUNDED);
                if nt.props.fixed && width > 0.0 {
                    (width, width)
                } else {
                    let style = table_style(ctx, nt);
                    let cols = column_content(ctx, nt, style.as_ref(), depth + 1).unwrap_or_default();
                    let (l, h) = cols.iter().fold((0.0, 0.0), |(l, h), c| (l + c.0, h + c.1));
                    (width.max(l), width.max(h))
                }
            }
            Block::Table(_) => (0.0, 0.0),
        };
        lo = lo.max(l);
        hi = hi.max(h);
    }
    (lo, hi)
}

/// The list level `p` is in, for its indents (without counting it: measuring isn't laying out).
fn list_level(ctx: &Ctx, p: &Paragraph) -> Option<wordcraft_doc::numbering::Level> {
    let n = p.props.numbering.or_else(|| ctx.doc.styles.resolve_para(&p.props).numbering).filter(|n| n.num != 0)?;
    ctx.doc.numbering.level(n.num, n.level).cloned()
}

/// A width to lay a paragraph out in so that only its own line breaks end lines.
const UNBOUNDED: f32 = 100_000.0;

/// A paragraph's longest word and longest line (between line breaks), indents included.
fn para_content(pl: &ParaLayout) -> (f32, f32) {
    let indents = pl.rp.indent_left.max(0.0) + pl.rp.indent_right.max(0.0) + pl.rp.indent_first.max(0.0);
    let (mut word, mut line, mut spaces) = (0.0f32, 0.0f32, 0.0f32);
    let (mut lo, mut hi) = (0.0f32, 0.0f32);
    for c in &pl.clusters {
        let adv = if c.adv.is_finite() { c.adv.clamp(0.0, UNBOUNDED) } else { 0.0 };
        match c.kind {
            ClKind::Space => {
                lo = lo.max(word);
                word = 0.0;
                spaces += adv;
            }
            ClKind::LineBreak | ClKind::PageBreak | ClKind::ColumnBreak => {
                lo = lo.max(word);
                hi = hi.max(line);
                (word, line, spaces) = (0.0, 0.0, 0.0);
            }
            _ => {
                word += adv;
                line += spaces + adv;
                spaces = 0.0;
                if c.break_after {
                    lo = lo.max(word);
                    word = 0.0;
                }
            }
        }
    }
    (lo.max(word) + indents, hi.max(line) + indents)
}

/// Whether nothing in `t` asks for a width (its own or any cell's): its columns then follow
/// their content alone.
fn is_automatic(t: &Table) -> bool {
    t.props.width.is_none()
        && t.props.width_pct.is_none()
        && t.rows.iter().all(|r| r.cells.iter().all(|c| c.props.width.is_none() && c.props.width_pct.is_none()))
}

/// Word's autofit for a table whose widths may follow its content (`tblLayout` not fixed).
/// - With no preferred width anywhere (the table's and every cell's automatic) the columns come
///   from their content alone: each its widest line if they all fit in `avail`, else shared out
///   between narrowest and widest in proportion to how much each can give.
/// - Otherwise the grid stands, but no column is narrower than its longest word: the columns with
///   room to spare give up the difference.
fn autofit(grid: &mut [f32], content: &[(f32, f32)], t: &Table, avail: f32) {
    if grid.len() != content.len() || !avail.is_finite() {
        return;
    }
    if is_automatic(t) {
        let (lo, hi) = content.iter().fold((0.0f32, 0.0f32), |(l, h), c| (l + c.0, h + c.1.max(c.0)));
        let k = if hi <= avail {
            1.0
        } else if lo >= avail || hi - lo < 0.01 {
            0.0
        } else {
            (avail - lo) / (hi - lo)
        };
        for (w, c) in grid.iter_mut().zip(content) {
            *w = (c.0 + (c.1.max(c.0) - c.0) * k).max(4.0);
        }
        return;
    }
    let short: f32 = grid.iter().zip(content).map(|(w, c)| (c.0 - *w).max(0.0)).sum();
    if short < 0.01 {
        return;
    }
    let spare: f32 = grid.iter().zip(content).map(|(w, c)| (*w - c.0).max(0.0)).sum();
    let give = (short / spare.max(0.01)).min(1.0);
    for (w, c) in grid.iter_mut().zip(content) {
        *w = if *w < c.0 { c.0 } else { *w - (*w - c.0) * give };
    }
}
