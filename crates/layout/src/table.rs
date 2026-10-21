//! Table layout: column widths, cell boxes, row heights, merged cells, borders and shading,
//! table-style conditional formatting. Rows are laid out as free-standing boxes; pagination
//! moves whole rows or splits them between lines (`split_row`).

use wordcraft_doc::numbering::Counters;
use wordcraft_doc::props::{Align, Border, Borders, HeightRule, Rgb, TextDirection, VAlign, VMerge};
use wordcraft_doc::styles::{PartSet, TablePart, TableStyleProps};
use wordcraft_doc::{Block, Blocks, Paragraph, StoryRef, Table};
use wordcraft_geom::Rect;

use crate::para::{CellText, ClKind, ParaLayout};
use crate::{BoxContext, BoxLayout, Ctx, Placed, layout_box};

/// Largest preferred table width, as a share of the text column, that a file can ask for.
const MAX_WIDTH_SHARE: f32 = 10.0;

pub struct RowLayout {
    pub height: f32,
    /// Minimum retained on continuation fragments that can hold it.
    pub(crate) min_height: f32,
    /// Items relative to the table's top-left (x) and the row's top (y).
    pub items: Vec<Placed>,
    /// The room the row keeps below its text: the bottom cell margin and border band. Part of a
    /// row split across pages keeps it too.
    pub bottom: f32,
    /// The row's cells: the items each one owns and where its content ends.
    pub cells: Vec<CellPart>,
}

/// A cell of a [`RowLayout`].
pub(crate) struct CellCenter {
    offset: f32,
    height: f32,
    content: std::ops::Range<usize>,
}

pub struct CellPart {
    pub(crate) center: Option<CellCenter>,
    /// Top cell margin and visible border band retained on a continuation.
    pub(crate) top: f32,
    /// Its items in the row's `items` (its fill, text, borders and cell area).
    pub items: std::ops::Range<usize>,
    /// Where its content ends below the row's top, its last paragraph's space after included
    /// (what the part of the row left on a page must hold); `None` for turned text and a cell
    /// covered by a vertical merge.
    pub end: Option<f32>,
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
    /// Its table style parts and cell index in the row.
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

/// The margins (top, left, bottom, right; points) of cell `cell` of row `row`: its own, else the
/// table's defaults. The layout uses the left and right ones as they are (the app's ruler
/// measures a cell's indents from them) but gives every cell in a row the row's largest top and
/// bottom margins.
pub fn cell_margins(doc: &wordcraft_doc::Document, t: &Table, row: usize, cell: usize) -> [f32; 4] {
    let style = t.props.style.as_deref().and_then(|id| doc.styles.table_style(id));
    let defaults = default_margins(t, style.as_ref());
    t.rows.get(row).and_then(|r| r.cells.get(cell)).map_or(defaults, |cell| cell.props.resolved_margins(defaults))
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
            let margins = cell.props.resolved_margins(margins_def);
            let side = margins[1].max(0.0) + margins[3].max(0.0);
            let text = style.as_ref().map(|st| region_text(st, region_of(t, style.as_ref(), ri, g, span, ncols)));
            let env = crate::para::ParaEnv {
                doc,
                width: MEASURE_WIDE,
                label: None,
                fields: &fields,
                show_hidden: false,
                field_codes: false,
                hide_deleted: Default::default(),
                table: text.as_ref(),
                proofing: None,
                typing_word: None,
                // Measurement uses a fixed environment (like show_hidden/proofing above).
                numeral: crate::NumeralMode::default(),
                exclusions: &[],
                eq_number: 0,
                read_line_spacing: None,
                grid: crate::para::Grid::default(),
                fit_objects: false,
                fitted: &[],
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
    t.rows.first().and_then(|r| r.cells.first()).map_or(def, |cell| cell.props.resolved_margins(def))[1]
}

/// What a table style gives the cells in one set of parts: shading, vertical alignment and each
/// part's cell borders, lowest precedence first.
struct RegionLook {
    fill: Option<Rgb>,
    v_align: Option<VAlign>,
    borders: Vec<(TablePart, Borders)>,
}

fn region_look(st: &TableStyleProps, region: Region) -> RegionLook {
    let mut out = RegionLook { fill: None, v_align: None, borders: Vec::new() };
    for part in region.iter() {
        let f = st.part(part);
        out.fill = f.fill.or(out.fill);
        out.v_align = f.v_align.or(out.v_align);
        // The whole table's borders are the table borders, applied on their own.
        if part != TablePart::WholeTable
            && let Some(b) = f.borders
        {
            out.borders.push((part, b));
        }
    }
    out
}

/// A cell's borders from the table style's parts it is in, later parts winning. Row parts give
/// their left/right edges at the table's sides and `inside_v` between cells; column parts their
/// top/bottom edges at the table's top and bottom and `between` between rows; corner cells their
/// four edges.
fn region_borders(parts: &[(TablePart, Borders)], (ri, nrows): (usize, usize), (first_g, last_g): (bool, bool)) -> Borders {
    let mut out = Borders::default();
    for (part, b) in parts {
        let b = *b;
        let mapped = if part.is_row() {
            Borders {
                top: b.top,
                bottom: b.bottom,
                left: if first_g { b.left } else { b.inside_v },
                right: if last_g { b.right } else { b.inside_v },
                between: None,
                inside_v: None,
            }
        } else if part.is_column() {
            Borders {
                top: if ri == 0 { b.top } else { b.between },
                bottom: if ri + 1 >= nrows { b.bottom } else { b.between },
                left: b.left,
                right: b.right,
                between: None,
                inside_v: None,
            }
        } else {
            Borders { between: None, inside_v: None, ..b }
        };
        out.overlay(&mapped);
    }
    out
}

pub fn layout_table(ctx: &mut Ctx, story: StoryRef, t: &Table, path: &[u32], avail: f32, depth: usize) -> TableLayout {
    let style = table_style(ctx, t);
    // Cell text formatting and looks per set of style parts, built once per table.
    let mut cell_text: Vec<(Region, CellText)> = Vec::new();
    let mut looks: Vec<(Region, RegionLook)> = Vec::new();
    let ncols = t.cols().max(1);
    // Column widths.
    let mut grid: Vec<f32> = if t.grid.len() == ncols {
        // Grid units can be narrower than a text box, including omitted row-end columns.
        t.grid
            .iter()
            .map(|&w| match w {
                w if !w.is_finite() => 72.0,
                w if w <= 0.0 => 4.0,
                w => w,
            })
            .collect()
    } else {
        vec![avail / ncols as f32; ncols]
    };
    apply_cell_widths(t, &mut grid);
    let mut total: f32 = grid.iter().sum();
    // A top-level table in an inherited header or footer takes its share of the defining
    // section's width (`Ctx::pct_base`).
    let pct_base = if depth <= 1 { ctx.pct_base.unwrap_or(avail) } else { avail };
    let target = match (t.props.width_pct, t.props.width) {
        // Word keeps a percentage past 100%; the cap only bounds hostile input.
        (Some(p), _) if p > 0.0 => Some(pct_base * (p / 100.0).min(MAX_WIDTH_SHARE)),
        (_, Some(w)) if w > 0.0 => Some(w),
        _ => None,
    };
    if let Some(tw) = target {
        if total > 0.0 && total < 1.0 {
            // Divide first: a tiny positive total can overflow the scale factor.
            grid.iter_mut().for_each(|w| *w = (*w / total) * tw);
        } else {
            let k = tw / total.max(1.0);
            grid.iter_mut().for_each(|w| *w *= k);
        }
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
    let mut tborders = style.as_ref().and_then(|s| s.parts.borders).unwrap_or_default();
    if let Some(own) = t.props.borders {
        tborders.overlay(&own);
    }
    let first_cell = t.rows.first().and_then(|r| r.cells.first());
    // A right-to-left table (`w:bidiVisual`) mirrors: its first column stands at the right, its
    // start alignment and indent are measured from the right margin, and each cell's start
    // (left) margin and border are on its right.
    let rtl = t.props.bidi_visual;
    // How far in from the start margin the table's edge sits when it is start-aligned.
    let start_offset = if ctx.doc.settings.compat_mode >= 15 {
        // The border is centred on the edge, so half of it sits outside: Word moves the table in
        // by that half.
        let border = first_cell.and_then(|c| c.props.borders.and_then(|b| b.left)).or(tborders.left);
        indent + border.filter(Border::is_visible).map_or(0.0, |b| b.width.clamp(0.0, 12.0) / 2.0)
    } else {
        indent - first_cell_left_margin_in(t, margins_def)
    };
    let x = match (t.props.align, rtl) {
        (Some(Align::Center), _) => (avail - total) / 2.0,
        // End alignment: the right margin, or the left one in a right-to-left table.
        (Some(Align::Right), false) => avail - total,
        (Some(Align::Right), true) => 0.0,
        (_, true) => avail - total - start_offset,
        (_, false) => start_offset,
    };
    let nrows = t.rows.len();
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
        /// Where the content ends below the cell's top (before vertical alignment).
        end: f32,
    }
    let mut rows: Vec<Vec<CellBox>> = Vec::with_capacity(nrows);
    let mut heights: Vec<f32> = Vec::with_capacity(nrows);
    let mut bottoms: Vec<f32> = Vec::with_capacity(nrows);
    let mut minimums = Vec::with_capacity(nrows);
    for (ri, row) in t.rows.iter().enumerate() {
        let mut g = 0usize;
        let mut cells = Vec::with_capacity(row.cells.len());
        let mut rh = 0.0f32;
        // The tallest cell margins plus border bands in the row.
        let mut row_insets = 0.0f32;
        let mut row_below = 0.0f32;
        // Every cell an empty one with a hidden end-of-cell mark: the row is only its borders.
        let mut collapsed = true;
        // Word gives every cell in a row the row's largest top and bottom margins: a cell without
        // its own lines up with a neighbour that has 5.65pt ones.
        let (row_top, row_bottom) = row
            .cells
            .iter()
            .map(|c| c.props.resolved_margins(margins_def))
            .fold((0.0f32, 0.0f32), |(top, bottom), m| (top.max(m[0]), bottom.max(m[2])));
        for (ci, cell) in row.cells.iter().enumerate() {
            let span = cell.span();
            let x0 = colx.get(g).copied().unwrap_or(acc);
            let x1 = colx.get((g + span).min(ncols)).copied().unwrap_or(acc);
            // Right to left: grid columns run from the right edge, and the start (left) margin
            // is on the cell's right.
            let (x0, x1) = if rtl { ((acc - x1).max(0.0), (acc - x0).max(0.0)) } else { (x0, x1) };
            let m = cell.props.resolved_margins(margins_def);
            let margins = if rtl { [row_top, m[3], row_bottom, m[1]] } else { [row_top, m[1], row_bottom, m[3]] };
            let cw = (x1 - x0 - margins[1] - margins[3]).max(4.0);
            let region = region_of(t, style.as_ref(), ri, g, span, ncols);
            // Conditional formatting, later parts winning (ECMA-376 §17.7.6): whole table, column
            // bands, row bands, first/last column, header/total row, corner cells.
            if let Some(st) = &style
                && !cell_text.iter().any(|(r, _)| *r == region)
            {
                cell_text.push((region, region_text(st, region)));
                looks.push((region, region_look(st, region)));
            }
            let look = looks.iter().find(|(r, _)| *r == region).map(|(_, l)| l);
            let fill = cell.props.shading.or(look.and_then(|l| l.fill));
            let text = cell_text.iter().find(|(r, _)| *r == region).map(|(_, c)| c);
            let mut cpath = path.to_vec();
            cpath.push(ri as u32);
            cpath.push(ci as u32);
            // Effective borders: cell > table (outer vs inside).
            let tb = tborders;
            let edge =
                |own: Option<Border>, outer: bool, outer_b: Option<Border>, inner_b: Option<Border>| own.or(if outer { outer_b } else { inner_b });
            let cb = cell.props.borders.unwrap_or_default();
            // A row's table property exceptions (`w:tblPrEx`) override the table's borders for its cells.
            let row_borders = row.props.exceptions.as_ref().and_then(|e| e.borders).unwrap_or_default();
            let (first_g, last_g) = (g == 0, g + span >= ncols);
            let rb = look.map(|l| region_borders(&l.borders, (ri, nrows), (first_g, last_g))).unwrap_or_default();
            let mut borders = Borders {
                top: edge(cb.top.or(if ri == 0 { row_borders.top } else { row_borders.between }).or(rb.top), ri == 0, tb.top, tb.between),
                bottom: edge(
                    cb.bottom.or(if ri + 1 == nrows { row_borders.bottom } else { row_borders.between }).or(rb.bottom),
                    ri + 1 == nrows,
                    tb.bottom,
                    tb.between,
                ),
                left: edge(
                    cb.left.or(if first_g { row_borders.left } else { row_borders.inside_v }).or(if first_g { rb.left } else { rb.inside_v }),
                    first_g,
                    tb.left,
                    tb.inside_v,
                ),
                right: edge(
                    cb.right.or(if last_g { row_borders.right } else { row_borders.inside_v }).or(if last_g { rb.right } else { rb.inside_v }),
                    last_g,
                    tb.right,
                    tb.inside_v,
                ),
                between: None,
                inside_v: None,
            };
            // The logical start (left) and end (right) borders, drawn mirrored in an RTL table.
            if rtl {
                std::mem::swap(&mut borders.left, &mut borders.right);
            }
            // Word keeps the cell's text clear of its top border (and the last row's of its bottom
            // border): the border's width sits on top of the cell margin.
            let band = |b: Option<Border>| b.filter(Border::is_visible).map_or(0.0, |b| b.width.clamp(0.0, 12.0));
            let (band_t, band_b) = (band(borders.top), if ri + 1 == nrows { band(borders.bottom) } else { 0.0 });
            // A shared edge painted by the row above still clears this cell's text and minimum.
            let band_t = if band_t > 0.0 || cell.props.vmerge == VMerge::Continue {
                band_t
            } else {
                rows.last()
                    .into_iter()
                    .flatten()
                    .filter(|above| above.g < g + span && g < above.g + above.span)
                    .filter(|above| {
                        above.vmerge == VMerge::None
                            || !t.cell_at_grid(ri, above.g).is_some_and(|ci| {
                                t.grid_col(ri, ci) == above.g && row.cells.get(ci).is_some_and(|c| c.props.vmerge == VMerge::Continue)
                            })
                    })
                    .map(|above| band(above.borders.bottom))
                    .fold(0.0, f32::max)
            };
            let insets = margins[0] + margins[2] + band_t + band_b;
            let turn = cell.props.text_direction;
            let mut turned = None;
            let (items, h) = if cell.props.vmerge == VMerge::Continue {
                (Vec::new(), 0.0)
            } else if turn.is_turned() {
                collapsed = false;
                // Turned text: its lines run along the cell's height. An exact row height sets
                // their length; otherwise they get the length the text needs unwrapped and the row
                // grows to fit (the second pass lays them out again if the row ends up taller).
                let snap = (ctx.counters.clone(), ctx.eq_count);
                let len = match (row.props.height, row.props.height_rule) {
                    (Some(h), HeightRule::Exact) if h > 0.0 => (h + row_bottom - insets).clamp(4.0, TURNED_LIMIT),
                    _ => {
                        let probe = layout_box(ctx, story, &cell.blocks, &cpath, TURNED_MAX, BoxContext::Cell(text), depth, None).items;
                        (ctx.counters, ctx.eq_count) = (snap.0.clone(), snap.1);
                        natural_length(&probe).clamp(4.0, TURNED_MAX)
                    }
                };
                let BoxLayout { items, height, floats_bottom, .. } =
                    layout_box(ctx, story, &cell.blocks, &cpath, len, BoxContext::Cell(text), depth, None);
                let across = height.max(floats_bottom);
                turned = Some(Turned { turn, len, across, snap, region, ci, top: margins[0] + band_t });
                (items, len)
            } else {
                let BoxLayout { mut items, mut height, floats_bottom, last_top, .. } =
                    layout_box(ctx, story, &cell.blocks, &cpath, cw, BoxContext::Cell(text), depth, None);
                // A hidden end-of-cell mark: an empty last paragraph takes no room (Word for
                // Windows: a row of empty such cells is only its borders).
                if cell.props.hide_mark && cell.blocks.last().and_then(|b| b.as_para()).is_some_and(|p| p.text.is_empty()) {
                    height = last_top;
                } else {
                    collapsed = false;
                }
                for it in &mut items {
                    it.translate(x0 + margins[1], margins[0] + band_t);
                }
                // The cell grows to hold its floating tables.
                (items, height.max(floats_bottom))
            };
            let h = h + insets;
            let end = h - margins[2] - band_b;
            if cell.props.vmerge != VMerge::Restart {
                rh = rh.max(h);
            }
            row_insets = row_insets.max(insets);
            row_below = row_below.max(margins[2] + band_b);
            cells.push(CellBox {
                items,
                h,
                x: x0,
                w: x1 - x0,
                // The style's vertical alignment unless the cell has its own.
                // The cell's own alignment, even Top, beats the table style's.
                valign: cell.props.valign.or_else(|| look.and_then(|l| l.v_align)).unwrap_or_default(),
                fill,
                borders,
                vmerge: cell.props.vmerge,
                margins,
                g,
                span,
                turned,
                end,
            });
            g += span;
        }
        // Word adds the cells' top and bottom margins and border bands to an at-least row height:
        // a 30pt row with 5pt margins and 0.5pt borders is 40.5pt tall even when its text needs less.
        // An exact height gains only the bottom margin: it runs from the top border to the bottom
        // of the text area (a 35pt row with 4pt margins steps 39pt).
        let rh = match (row.props.height, row.props.height_rule) {
            (Some(h), HeightRule::Exact) if h > 0.0 => h + row_bottom,
            (Some(h), _) if h > 0.0 => rh.max(h + row_insets),
            _ => rh,
        };
        minimums.push(match (row.props.height, row.props.height_rule) {
            (Some(h), HeightRule::AtLeast) if h > 0.0 => h + row_insets,
            _ => 0.0,
        });
        bottoms.push(row_below);
        heights.push(if collapsed && !cells.is_empty() { rh.max(0.0) } else { rh.max(4.0) });
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
        let exact = t.rows.get(ri).is_some_and(|r| r.props.height_rule == HeightRule::Exact && r.props.height.is_some_and(|h| h > 0.0));
        let mut items = Vec::new();
        let mut parts = Vec::with_capacity(cells.len());
        for (ci, c) in cells.into_iter().enumerate() {
            let first = items.len();
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
            // Behind-text frames count towards vertical alignment even without inline content.
            // They do not increase the row's flow height.
            let align_h = c
                .items
                .iter()
                .filter_map(|item| match item {
                    Placed::Object { rect, wrap: wordcraft_doc::para::Wrap::BehindText, .. } => Some(rect.bottom()),
                    _ => None,
                })
                .fold(c.h, f32::max);
            let mut dy = match c.valign {
                VAlign::Top => 0.0,
                VAlign::Center => ((span_h - align_h) / 2.0).max(0.0),
                VAlign::Bottom => (span_h - align_h).max(0.0),
            };
            let end = (c.vmerge != VMerge::Continue && c.turned.is_none()).then_some(c.end + dy);
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
                    let b = layout_box(ctx, story, &cell.blocks, &cpath, len, BoxContext::Cell(text), depth, None);
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
            let content_start = items.len();
            for mut it in cell_items {
                it.translate(0.0, dy);
                items.push(it);
            }
            let center = (c.vmerge == VMerge::Restart && c.valign == VAlign::Center && !exact && dy > 0.0).then_some(CellCenter {
                offset: dy,
                height: align_h,
                content: content_start..items.len(),
            });
            let b = c.borders;
            let (x0, x1, y0, y1) = (c.x, c.x + c.w, 0.0, rh);
            // Horizontal edges run on over half of each visible vertical edge, so the corners
            // close the way Word draws them instead of leaving a notch at thick borders.
            let half = |o: Option<Border>| o.filter(Border::is_visible).map_or(0.0, |e| e.width / 2.0);
            let (hx0, hx1) = (x0 - half(b.left), x1 + half(b.right));
            let top_vis = c.vmerge != VMerge::Continue;
            if top_vis && let Some(e) = b.top.filter(Border::is_visible) {
                items.push(Placed::Rule { x0: hx0, y0, x1: hx1, y1: y0, border: e });
            }
            let last_of_merge = c.vmerge == VMerge::None || !cont.get(ri + 1).is_some_and(|v| v.contains(&c.g));
            if last_of_merge && let Some(e) = b.bottom.filter(Border::is_visible) {
                items.push(Placed::Rule { x0: hx0, y0: y1, x1: hx1, y1, border: e });
            }
            if let Some(e) = b.left.filter(Border::is_visible) {
                items.push(Placed::Rule { x0, y0, x1: x0, y1, border: e });
            }
            if let Some(e) = b.right.filter(Border::is_visible) {
                items.push(Placed::Rule { x0: x1, y0, x1, y1, border: e });
            }
            items.push(Placed::Cell {
                rect: Rect::new(c.x, 0.0, c.w, if c.vmerge == VMerge::Continue { rh } else { span_h }),
                covered: c.vmerge == VMerge::Continue,
                table: wordcraft_doc::Path(path.to_vec()),
                row: ri,
                cell: ci,
                story,
                clip: exact && c.h > span_h + 0.01,
            });
            let top = c.margins[0] + c.borders.top.filter(Border::is_visible).map_or(0.0, |b| b.width.clamp(0.0, 12.0));
            parts.push(CellPart { top, center, items: first..items.len(), end });
        }
        out.push(RowLayout {
            height: rh,
            min_height: minimums.get(ri).copied().unwrap_or(0.0),
            items,
            bottom: bottoms.get(ri).copied().unwrap_or(0.0),
            cells: parts,
        });
    }
    TableLayout { x, width: total, rows: out }
}

/// Split a row at `cut` (points below the row's top) so the part above fits on this page.
/// Paragraph lines break between lines; fills, vertical rules and cell areas are cut; the
/// rest moves up to the top of the continuation. `None` when no line fits above the cut.
///
/// Like Word, the part left on the page keeps the cells' bottom margin and border below its
/// text and a cell's last paragraph its space after (a row only that much taller than the room
/// left splits rather than moving on whole), and widow control keeps two lines of a paragraph on
/// either side of the break. Each cell's text goes on at the top of the continuation, and the
/// continuation is as tall as the cell needing the most room, and at least the rest of the row
/// (a minimum height holds over the pages).
pub fn split_row(row: &RowLayout, cut: f32) -> Option<(RowLayout, RowLayout)> {
    // Also rejects a NaN cut.
    if !(cut > 0.0 && cut < row.height) {
        return None;
    }
    let limit = cut - row.bottom.clamp(0.0, cut);
    let fits = |bottom: f32| bottom <= limit + 0.01;
    // Each cell's items: every item of a row is in one of its cells.
    let cells: Vec<&[Placed]> = row.cells.iter().map(|c| row.items.get(c.items.clone()).unwrap_or(&[])).collect();
    // A nested table's cell is deeper than the row's own.
    let own_depth = row.items.iter().filter_map(|it| if let Placed::Cell { table, .. } = it { Some(table.0.len()) } else { None }).min();
    // The first line of each paragraph that moves on (`l1` when all of them stay), by cell and
    // item.
    let splits: Vec<Vec<usize>> = cells
        .iter()
        .zip(&row.cells)
        .map(|(items, c)| {
            // Where the cell's text ends: the paragraph ending lowest is the cell's last.
            let lowest =
                items.iter().filter_map(|it| text_lines(it).and_then(|(p, l0, l1, y)| crate::item_bottom(y, p, l0, l1))).fold(f32::MIN, f32::max);
            items
                .iter()
                .map(|it| {
                    let Some((para, l0, l1, y)) = text_lines(it) else { return 0 };
                    // The cell's last paragraph ends with the cell's content (its space after).
                    let last = l1 == para.lines.len() && crate::item_bottom(y, para, l0, l1).is_some_and(|b| b >= lowest - 0.01);
                    line_split(para, (l0, l1), y, limit, c.end.filter(|_| last))
                })
                .collect()
        })
        .collect();
    // A cell that widow/orphan control leaves with nothing on this page, though its first line
    // would fit there, takes the whole row on with it: Word doesn't leave the other cells behind
    // beside an empty one (observed with synthetic documents). Trailing space alone can move
    // the last paragraph without deferring the whole row.
    for (items, splits) in cells.iter().zip(&splits) {
        let first = items.iter().zip(splits).find_map(|(it, &split)| text_lines(it).map(|t| (t, split)));
        if let Some(((para, l0, l1, y), split)) = first
            && split == l0
            && para.rp.widow_control
            && l1.saturating_sub(l0) > 1
            && crate::item_bottom(y, para, l0, l1).is_some_and(|bottom| !fits(bottom))
            && para.lines.get(l0).is_some_and(|l| fits(y + l.top - para.lines.get(l0).map_or(0.0, |f| f.top) + l.height))
        {
            return None;
        }
    }
    // The top of the first thing that moves on in each cell.
    let mut kept_any = false;
    let mut moved = vec![f32::MAX; cells.len()];
    for ((items, splits), m) in cells.iter().zip(&splits).zip(&mut moved) {
        for (it, &split) in items.iter().zip(splits) {
            let top = if let Some(r) = it.turned_bounds() {
                // Turned text moves as a whole, like a picture.
                if fits(r.bottom()) {
                    kept_any = true;
                    None
                } else {
                    Some(r.y)
                }
            } else {
                match it {
                    Placed::Lines { para, l0, l1, y, .. } => {
                        kept_any |= split > *l0;
                        let base = para.lines.get(*l0).map(|l| l.top).unwrap_or(0.0);
                        (split < *l1).then(|| para.lines.get(split).map_or(*y, |l| y + l.top - base))
                    }
                    Placed::Image { rect, .. } | Placed::Shape { rect, .. } | Placed::Graphic { rect, .. } | Placed::Clip { rect, .. } => {
                        if fits(rect.bottom()) {
                            kept_any = true;
                            None
                        } else {
                            Some(rect.y)
                        }
                    }
                    // A nested table's cell the cut runs through goes on from the cut, so what is
                    // below the cut keeps its size and the text after it stays below it (a
                    // floating table's fixed-height row in an otherwise fitting cell splits the
                    // row there). A row that is just taller than its text (a minimum height)
                    // moves on whole, as in Word.
                    Placed::Cell { rect, table, .. } if Some(table.0.len()) > own_depth && rect.y < cut && rect.bottom() > cut + 0.01 => Some(cut),
                    _ => None,
                }
            };
            if let Some(top) = top {
                *m = m.min(top);
            }
        }
    }
    let first_moved = moved.iter().copied().fold(f32::MAX, f32::min);
    if !kept_any || first_moved == f32::MAX {
        return None;
    }
    // Word closes the first fragment below its last fitted content.
    // Nested boxes crossing the cut retain their geometry.
    let mut kept_bottoms = Vec::with_capacity(cells.len());
    for (((items, splits), c), &moved_top) in cells.iter().zip(&splits).zip(&row.cells).zip(&moved) {
        let mut kept_bottom = 0.0f32;
        for (it, &split) in items.iter().zip(splits) {
            let bottom = if let Some(r) = it.turned_bounds() {
                fits(r.bottom()).then_some(r.bottom())
            } else {
                match it {
                    Placed::Lines { para, l0, y, .. } if split > *l0 => crate::item_bottom(*y, para, *l0, split),
                    Placed::Image { rect, .. } | Placed::Shape { rect, .. } | Placed::Graphic { rect, .. } | Placed::Clip { rect, .. }
                        if fits(rect.bottom()) =>
                    {
                        Some(rect.bottom())
                    }
                    Placed::Object { rect, wrap, .. } if *wrap != wordcraft_doc::para::Wrap::BehindText && fits(rect.bottom()) => Some(rect.bottom()),
                    Placed::Cell { rect, table, .. } if Some(table.0.len()) > own_depth && rect.y < cut => Some(rect.bottom().min(cut)),
                    _ => None,
                }
            };
            if let Some(bottom) = bottom {
                kept_bottom = kept_bottom.max(bottom);
            }
        }
        if moved_top == f32::MAX
            && let Some(end) = c.end.filter(|e| fits(*e))
        {
            kept_bottom = kept_bottom.max(end);
        }
        kept_bottoms.push(kept_bottom);
    }
    let floor = if row.min_height <= cut + 0.01 { row.min_height } else { cut };
    let plain_bottom = kept_bottoms.iter().zip(&row.cells).filter(|(_, c)| c.center.is_none()).map(|(b, _)| *b).fold(0.0f32, f32::max);
    let plain_height = (plain_bottom + row.bottom).max(floor).min(cut);
    // A fitting merged label uses the first physical fragment's vertical center.
    let kept_bottom = kept_bottoms
        .iter()
        .zip(&row.cells)
        .filter(|(_, c)| c.center.as_ref().is_none_or(|v| v.height > plain_height + 0.01))
        .map(|(b, _)| *b)
        .fold(0.0f32, f32::max);
    let first_height = (kept_bottom + row.bottom).max(floor).min(cut);
    let trim_first = |r: Rect| (r.y < first_height).then(|| Rect::new(r.x, r.y, r.w, r.h.min(first_height - r.y)));
    // Each cell resumes below its own top margin and border band.
    let shifts: Vec<f32> = moved.iter().zip(&row.cells).map(|(&m, c)| ((if m < f32::MAX { m } else { first_moved }) - c.top).max(0.0)).collect();
    // The continuation is as tall as the cell needing the most room (what it moves on and the
    // bottom margin, the rest of the row for turned text), and at least the rest of the row.
    let full = (row.height - shifts.iter().copied().fold(f32::MAX, f32::min)).max(4.0);
    let mut need = row.height - cut;
    for ((c, &m), &s) in row.cells.iter().zip(&moved).zip(&shifts) {
        if m < f32::MAX {
            need = need.max(c.end.map_or(row.height, |e| e + row.bottom) - s);
        }
    }
    // Word reapplies an at-least height on the continuation. Oversized minimums
    // that cannot fit the first fragment keep their existing distributed height.
    let min_height = if row.min_height <= cut + 0.01 { row.min_height } else { 0.0 };
    let height = need.clamp(4.0, full).max(min_height);
    // What runs to the row's bottom (cell areas, shading, side borders) ends at the
    // continuation's.
    let rest = (row.height - height).max(0.0);
    let geo_shift = |bottom: f32, shift: f32| if bottom >= row.height - 0.01 { rest } else { shift };
    let (mut a, mut b) = (Vec::new(), Vec::new());
    let mut parts = Vec::with_capacity(cells.len());
    for (((c, items), splits), &shift) in row.cells.iter().zip(&cells).zip(&splits).zip(&shifts) {
        let start = b.len();
        let center = c.center.as_ref().filter(|v| v.height <= plain_height + 0.01);
        for (index, (it, &split)) in items.iter().zip(splits).enumerate() {
            if let Some(center) = center
                && center.content.contains(&(c.items.start + index))
            {
                let mut it = it.clone();
                it.translate(0.0, ((first_height - center.height) / 2.0).max(0.0) - center.offset);
                a.push(it);
                continue;
            }
            if let Some(r) = it.turned_bounds() {
                let mut it = it.clone();
                if fits(r.bottom()) {
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
                    if split > *l0 {
                        a.push(Placed::Lines {
                            story: *story,
                            path: path.clone(),
                            para: para.clone(),
                            l0: *l0,
                            l1: split,
                            x: *x,
                            y: *y,
                            turn: *turn,
                        });
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
                Placed::Image { rect, .. }
                | Placed::Shape { rect, .. }
                | Placed::Graphic { rect, .. }
                | Placed::Clip { rect, .. }
                | Placed::Object { rect, .. } => {
                    if fits(rect.bottom()) {
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
                    let (above, below) = split_rect(*rect, cut, geo_shift(rect.bottom(), shift));
                    a.extend(above.and_then(trim_first).map(|rect| Placed::Fill { rect, color: *color }));
                    b.extend(below.map(|rect| Placed::Fill { rect, color: *color }));
                }
                Placed::Cell { rect, covered, table, row: r, cell, story, clip } => {
                    let (above, below) = split_rect(*rect, cut, geo_shift(rect.bottom(), shift));
                    let cell_at =
                        |rect| Placed::Cell { rect, covered: *covered, table: table.clone(), row: *r, cell: *cell, story: *story, clip: *clip };
                    a.extend(above.and_then(trim_first).map(cell_at));
                    b.extend(below.map(cell_at));
                }
                Placed::Rule { x0, y0, x1, y1, border } => {
                    if (y0 - y1).abs() < 0.01 {
                        // Horizontal: the top edge stays, the bottom edge moves; both fragments close.
                        if *y0 <= 0.01 {
                            a.push(it.clone());
                            b.push(Placed::Rule { x0: *x0, y0: 0.0, x1: *x1, y1: 0.0, border: *border });
                        } else if *y0 >= row.height - 0.01 {
                            a.push(Placed::Rule { x0: *x0, y0: first_height, x1: *x1, y1: first_height, border: *border });
                            b.push(Placed::Rule { x0: *x0, y0: y0 - rest, x1: *x1, y1: y1 - rest, border: *border });
                        } else if *y0 <= cut {
                            a.push(it.clone());
                        } else {
                            b.push(Placed::Rule { x0: *x0, y0: y0 - shift, x1: *x1, y1: y1 - shift, border: *border });
                        }
                    } else {
                        let (t, bt) = (y0.min(*y1), y0.max(*y1));
                        if t < first_height {
                            a.push(Placed::Rule { x0: *x0, y0: t, x1: *x1, y1: bt.min(first_height), border: *border });
                        }
                        if bt > cut {
                            let shift = geo_shift(bt, shift);
                            b.push(Placed::Rule { x0: *x0, y0: (t - shift).max(0.0), x1: *x1, y1: bt - shift, border: *border });
                        }
                    }
                }
            }
        }
        parts.push(CellPart { top: c.top, items: start..b.len(), end: c.end.filter(|_| center.is_none()).map(|e| e - shift), center: None });
    }
    // The part left on the page is placed and never split again.
    Some((
        RowLayout { height: first_height, min_height: 0.0, items: a, bottom: row.bottom, cells: Vec::new() },
        RowLayout { height, min_height, items: b, bottom: row.bottom, cells: parts },
    ))
}

/// A cell paragraph's placed lines (not turned text): the paragraph, its lines `l0..l1` and the
/// top of line `l0`.
fn text_lines(it: &Placed) -> Option<(&ParaLayout, usize, usize, f32)> {
    match it {
        Placed::Lines { para, l0, l1, y, .. } if it.turned_bounds().is_none() => Some((para, *l0, *l1, *y)),
        _ => None,
    }
}

/// The first of lines `l0..l1` of `para` (placed with line `l0`'s top at `y`) that does not fit
/// above `limit`: the last line of a cell's last paragraph needs room down to the cell's content
/// `end`, its space after. Widow control keeps two lines on either side of the break, or moves
/// the paragraph on whole.
fn line_split(para: &ParaLayout, (l0, l1): (usize, usize), y: f32, limit: f32, end: Option<f32>) -> usize {
    let n = para.lines.len();
    let mut k = l0;
    while k < l1 {
        let Some(bottom) = crate::item_bottom(y, para, l0, k + 1) else { break };
        let bottom = if k + 1 == n { end.map_or(bottom, |e| e.max(bottom)) } else { bottom };
        if bottom > limit + 0.01 {
            break;
        }
        k += 1;
    }
    // The body's rule differs: there a widow takes a line along only when three or more lines are
    // on the page (`place_para_in_flow`).
    if para.rp.widow_control && k > l0 && k < l1 {
        // A widow: take one more line along.
        if n - k == 1 {
            k -= 1;
        }
        // An orphan (the first line alone): the paragraph moves on whole.
        if l0 == 0 && k == 1 {
            k = 0;
        }
    }
    k
}

/// The parts of `r` above `cut` and below it, the latter moved up by `shift` (onto the
/// continuation of a row split at `cut`).
fn split_rect(r: Rect, cut: f32, shift: f32) -> (Option<Rect>, Option<Rect>) {
    let above = (r.y < cut).then(|| Rect::new(r.x, r.y, r.w, (r.bottom().min(cut) - r.y).max(0.0)));
    let top = (r.y - shift).max(0.0);
    let below = (r.bottom() > cut).then(|| Rect::new(r.x, top, r.w, (r.bottom() - shift - top).max(0.0)));
    (above, below)
}

/// A cell's table style parts.
type Region = PartSet;

/// The parts of the cell at grid column `g` spanning `span` of `ncols` columns in row `ri`.
fn region_of(t: &Table, style: Option<&TableStyleProps>, ri: usize, g: usize, span: usize, ncols: usize) -> Region {
    let size = style.and_then(|s| s.parts.band_size).unwrap_or(1);
    PartSet::of_cell(&t.props.look, size, (ri, t.rows.len()), (g, span, ncols))
}

/// The table style's text formatting in `region`: each part's character formatting and paragraph
/// alignment over the last, lowest precedence first (ECMA-376 §17.7.6).
fn region_text(st: &TableStyleProps, region: Region) -> CellText {
    let mut chr = wordcraft_doc::CharProps::default();
    let mut para = st.para.clone();
    for part in region.iter() {
        let f = st.part(part);
        chr.overlay(&f.chr);
        if f.align.is_some() {
            para.align = f.align;
        }
    }
    CellText { para, chr }
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
                let m = cell.props.resolved_margins(margins_def);
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
                ctx.para_measured(p, UNBOUNDED, text, label, para_content)
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

/// Consistent cell widths replace a stale grid without changing the table's total width.
/// Conflicting widths, spans and percentages need the full shared-grid algorithm.
fn apply_cell_widths(t: &Table, grid: &mut [f32]) {
    let Some(first) = t.rows.first() else { return };
    if first.cells.len() != grid.len() {
        return;
    }
    let widths: Option<Vec<f32>> = first.cells.iter().map(|c| c.props.width.filter(|w| w.is_finite() && *w > 0.0)).collect();
    let Some(widths) = widths else { return };
    let total: f32 = widths.iter().sum();
    if !total.is_finite() || (total - grid.iter().sum::<f32>()).abs() > 0.01 {
        return;
    }
    if t.rows.iter().any(|r| {
        r.cells.len() != grid.len()
            || r.cells.iter().zip(&widths).any(|(c, w)| c.span() != 1 || c.props.width_pct.is_some() || c.props.width != Some(*w))
    }) {
        return;
    }
    grid.copy_from_slice(&widths);
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
    // Donors give equal widths until they reach their own content minimum.
    let mut rooms: Vec<_> = grid.iter().zip(content).map(|(w, c)| *w - c.0).filter(|room| *room > 0.0).collect();
    rooms.sort_by(f32::total_cmp);
    let mut remaining = short;
    let mut share = 0.0;
    for (i, room) in rooms.iter().enumerate() {
        share = remaining / (rooms.len() - i) as f32;
        if share <= *room {
            break;
        }
        remaining -= room;
    }
    for (w, c) in grid.iter_mut().zip(content) {
        *w = if *w < c.0 { c.0 } else { *w - (*w - c.0).min(share) };
    }
}

#[cfg(test)]
mod width_tests {
    use super::*;

    #[test]
    fn autofit_shares_donor_loss() {
        let table = Table::new(1, 4, 260.0);
        let mut grid = [30.0, 120.0, 20.0, 90.0];
        let content = [(28.0, 28.0), (40.0, 120.0), (40.0, 40.0), (30.0, 90.0)];
        autofit(&mut grid, &content, &table, 260.0);
        assert_eq!(grid, [28.0, 111.0, 40.0, 81.0]);
    }

    #[test]
    fn autofit_exhausts_donors() {
        let table = Table::new(1, 2, 50.0);
        let mut grid = [30.0, 20.0];
        autofit(&mut grid, &[(25.0, 25.0), (40.0, 40.0)], &table, 50.0);
        assert_eq!(grid, [25.0, 40.0]);
    }

    #[test]
    fn autofit_keeps_fitting_widths() {
        let table = Table::new(1, 3, 120.0);
        let mut grid = [30.0, 40.0, 50.0];
        autofit(&mut grid, &[(10.0, 10.0), (20.0, 20.0), (30.0, 30.0)], &table, 120.0);
        assert_eq!(grid, [30.0, 40.0, 50.0]);
    }
}
