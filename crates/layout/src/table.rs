//! Table layout: column widths, cell boxes, row heights, merged cells, borders and shading,
//! table-style conditional formatting. Rows are laid out as free-standing boxes; pagination
//! moves whole rows or splits them between lines (`split_row`).

use wordcraft_doc::numbering::Counters;
use wordcraft_doc::props::{Align, Border, Borders, HeightRule, Rgb, TextDirection, VAlign, VMerge};
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
    /// Its formatting region (header row, total row, first column, row band) and cell index in the row.
    region: (bool, bool, bool, bool),
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

/// A cell region's text formatting from the table style: the whole table's, then column, then
/// row conditional formatting (later regions win, ECMA-376 §17.7.6). `region` is (header row,
/// total row, first column, row band).
fn region_text(st: &TableStyleProps, region: (bool, bool, bool, bool)) -> CellText {
    let (is_header, is_total, first_col, band) = region;
    let mut chr = st.chr.clone();
    if band {
        chr.overlay(&st.parts.band_chr);
    }
    if first_col {
        chr.overlay(&st.parts.first_col_chr);
    }
    if is_header {
        chr.overlay(&st.parts.header_chr);
    } else if is_total {
        chr.overlay(&st.parts.total_chr);
    }
    CellText { para: st.para.clone(), chr }
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
    let parts = style.as_ref().map(|s| &s.parts);
    let margins_def = default_margins(t, style.as_ref());
    let ncols = t.cols().clamp(1, wordcraft_doc::table::MAX_COLS);
    let fields = crate::FieldCtx::default();
    let header_rows = t.props.look.header_row;
    let band_size = parts.and_then(|p| p.band_size).unwrap_or(1).clamp(1, 1000) as usize;
    let nrows = t.rows.len();
    let mut cols = vec![(0.0f32, 0.0f32); ncols];
    // Cells spanning several columns: (first column, span, min, max), applied after the others.
    let mut spanned: Vec<(usize, usize, f32, f32)> = Vec::new();
    let mut budget = MEASURE_BUDGET;
    'rows: for (ri, row) in t.rows.iter().enumerate() {
        let is_header = header_rows && ri == 0;
        let is_total = t.props.look.total_row && ri + 1 == nrows && nrows > 1;
        let band = t.props.look.banded_rows && !is_header && (ri.saturating_sub(usize::from(header_rows)) / band_size).is_multiple_of(2);
        let mut g = 0usize;
        for cell in &row.cells {
            let span = cell.span();
            let margins = cell.props.margins.unwrap_or(margins_def);
            let side = margins[1].max(0.0) + margins[3].max(0.0);
            let text = style.as_ref().map(|st| region_text(st, (is_header, is_total, t.props.look.first_column && g == 0, band)));
            let env = crate::para::ParaEnv {
                doc,
                width: MEASURE_WIDE,
                label: None,
                fields: &fields,
                show_hidden: false,
                hide_deleted: false,
                table: text.as_ref(),
                proofing: false,
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

pub fn layout_table(ctx: &mut Ctx, story: StoryRef, t: &Table, path: &[u32], avail: f32, depth: usize) -> TableLayout {
    let style = table_style(ctx, t);
    let parts = style.as_ref().map(|s| &s.parts);
    // Cell text formatting per (header row, total row, first column, row band) region, built once
    // per table.
    let mut cell_text: Vec<((bool, bool, bool, bool), CellText)> = Vec::new();
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
    let band_size = parts.and_then(|p| p.band_size).unwrap_or(1).clamp(1, 1000) as usize;
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
        let band = t.props.look.banded_rows && !is_header && (ri.saturating_sub(usize::from(header_rows)) / band_size).is_multiple_of(2);
        // The header row's or the odd band's own cell borders (conditional formatting).
        let region_borders = if is_header {
            parts.and_then(|p| p.header_borders)
        } else if band {
            parts.and_then(|p| p.band_borders)
        } else {
            None
        }
        .unwrap_or_default();
        for (ci, cell) in row.cells.iter().enumerate() {
            let span = cell.span();
            let x0 = colx.get(g).copied().unwrap_or(acc);
            let x1 = colx.get((g + span).min(ncols)).copied().unwrap_or(acc);
            let margins = cell.props.margins.unwrap_or(margins_def);
            let cw = (x1 - x0 - margins[1] - margins[3]).max(4.0);
            let mut fill = cell.props.shading;
            let region = (is_header, is_total, t.props.look.first_column && g == 0, band);
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
            let rb = region_borders;
            let (first_g, last_g) = (g == 0, g + span >= ncols);
            let mut borders = Borders {
                top: edge(cb.top.or(rb.top), ri == 0, tb.top, tb.between),
                bottom: edge(cb.bottom.or(rb.bottom), ri + 1 == nrows, tb.bottom, tb.between),
                left: edge(cb.left.or(if first_g { rb.left } else { rb.inside_v }), first_g, tb.left, tb.inside_v),
                right: edge(cb.right.or(if last_g { rb.right } else { rb.inside_v }), last_g, tb.right, tb.inside_v),
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
                        let (probe, _) = layout_box(ctx, story, &cell.blocks, &cpath, TURNED_MAX, text, depth, None);
                        (ctx.counters, ctx.eq_count) = (snap.0.clone(), snap.1);
                        natural_length(&probe).clamp(4.0, TURNED_MAX)
                    }
                };
                let (items, across) = layout_box(ctx, story, &cell.blocks, &cpath, len, text, depth, None);
                turned = Some(Turned { turn, len, across, snap, region, ci, top: margins[0] + band_t });
                (items, len)
            } else {
                let (mut items, h) = layout_box(ctx, story, &cell.blocks, &cpath, cw, text, depth, None);
                for it in &mut items {
                    it.translate(x0 + margins[1], margins[0] + band_t);
                }
                (items, h)
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
                    (cell_items, across) = layout_box(ctx, story, &cell.blocks, &cpath, len, text, depth, None);
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
