//! Table Design › Borders: the border pen (Border Styles) and the Border Painter. The pen is a
//! line style, width and colour held in the session, not the document; `table.borders` draws with
//! it when no style, width or colour is given. The app owns the painter mode (a canvas tool, like
//! Draw Table and the Eraser): each click on a border runs `table.borderPainter` with page
//! coordinates, which paints just that border with the pen (one undo step).

use serde_json::{Value, json};
use wordcraft_doc::props::{Border, BorderStyle, Rgb, VMerge};
use wordcraft_doc::{Path, StoryRef, Table};
use wordcraft_layout::Placed;

use super::{sel_result, table};
use crate::{CmdError, CmdResult, CommandSpec, Session, p};

/// How far from a cell's edge (points) a click still paints that edge.
const REACH: f32 = 5.0;

/// The pen borders are drawn with: default single ½ pt, automatic colour.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BorderPen {
    pub style: BorderStyle,
    /// Points, 0.25–6.
    pub width: f32,
    /// `None` = automatic.
    pub color: Option<Rgb>,
}

impl Default for BorderPen {
    fn default() -> Self {
        BorderPen { style: BorderStyle::Single, width: 0.5, color: None }
    }
}

impl BorderPen {
    pub fn border(self) -> Border {
        Border { style: self.style, width: self.width, color: self.color, space: 0.0 }
    }

    pub fn json(self) -> Value {
        json!({"style": self.style.ooxml(), "width": self.width, "color": self.color.map(Rgb::hex)})
    }
}

/// A pen's line style by name; "none" and unknown names are errors.
pub fn parse_style(name: &str) -> Result<BorderStyle, CmdError> {
    match name {
        "single" => Ok(BorderStyle::Single),
        "double" => Ok(BorderStyle::Double),
        "dotted" => Ok(BorderStyle::Dotted),
        "dashed" => Ok(BorderStyle::Dashed),
        "thick" => Ok(BorderStyle::Thick),
        "triple" => Ok(BorderStyle::Triple),
        "dotDash" => Ok(BorderStyle::DotDash),
        "wave" => Ok(BorderStyle::Wave),
        _ => Err(CmdError::Params(format!("unknown border style `{name}` (single, double, dotted, dashed, thick, triple, dotDash or wave)"))),
    }
}

/// A width in points, clamped to what a border can be.
pub fn clamp_width(w: f32) -> f32 {
    w.clamp(0.25, 6.0)
}

/// A `color` parameter: `None` when absent; `Some(None)` for automatic (null, "" or "auto").
pub(crate) fn color_param(v: &Value) -> Result<Option<Option<Rgb>>, CmdError> {
    match v.get("color") {
        None => Ok(None),
        Some(Value::Null) => Ok(Some(None)),
        Some(Value::String(c)) if c.is_empty() || c.eq_ignore_ascii_case("auto") => Ok(Some(None)),
        Some(Value::String(c)) => {
            Rgb::parse(c).map(|c| Some(Some(c))).ok_or_else(|| CmdError::Params(format!("`color` {c:?} isn't RRGGBB or \"auto\"")))
        }
        Some(_) => Err(CmdError::Params("`color` is \"RRGGBB\", \"auto\" or null".into())),
    }
}

pub fn specs() -> Vec<CommandSpec> {
    vec![
        CommandSpec::new("table.borderStyles", "Border Styles", "Table Design › Borders", border_styles)
            .params(r#"{"style"?: "single|double|dotted|dashed|thick|triple|dotDash|wave", "width"?: pt (0.25–6), "color"?: "RRGGBB" | "auto" | null} (sets the border pen that Borders and Border Painter draw with; returns the pen. In the app, picking a style also turns on Border Painter.)"#)
            .pure(),
        CommandSpec::new("table.borderPainter", "Border Painter", "Table Design › Borders", border_painter).params(
            r#"{"page": n, "x": pt, "y": pt} (a click on a table border in page points, 0-based page: paints that border with the pen, along every cell it runs past, a merged cell's whole side) | {"kind"?: "all|outside|inside|none|top|bottom|left|right", "style"?, "width"?, "color"?} (headless: those borders of the caret's table, default all, with the pen; style/width/color as on table.borders win over it). In the app (and over --connect) a call without coordinates or kind, e.g. {} or {"on": bool}, turns the Border Painter mode on or off instead; pass kind to paint borders."#,
        ),
    ]
}

fn border_styles(s: &mut Session, v: &Value) -> CmdResult {
    let mut pen = s.border_pen;
    if let Some(name) = p::str(v, "style") {
        pen.style = parse_style(name)?;
    }
    if let Some(w) = p::f32(v, "width") {
        pen.width = clamp_width(w);
    }
    if let Some(c) = color_param(v)? {
        pen.color = c;
    }
    s.border_pen = pen;
    Ok(pen.json())
}

fn border_painter(s: &mut Session, v: &Value) -> CmdResult {
    if v.get("page").is_some() || v.get("x").is_some() || v.get("y").is_some() {
        return paint_at(s, v);
    }
    let mut args = json!({"kind": p::str(v, "kind").unwrap_or("all")});
    for k in ["style", "width", "color"] {
        if let (Some(x), Some(o)) = (v.get(k), args.as_object_mut()) {
            o.insert(k.into(), x.clone());
        }
    }
    s.run("table.borders", &args)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Side {
    Top,
    Bottom,
    Left,
    Right,
}

/// The cell edge a click at (`x`, `y`) on `page` is on: the nearest edge within [`REACH`] of a
/// cell around the point (a nested table's cell before the one holding it).
fn edge_at(l: &wordcraft_layout::DocLayout, page: usize, x: f32, y: f32) -> Option<(StoryRef, Path, usize, usize, Side, wordcraft_geom::Rect)> {
    let pg = l.pages.get(page)?;
    let mut best: Option<(f32, usize, (StoryRef, Path, usize, usize, Side, wordcraft_geom::Rect))> = None;
    for it in &pg.items {
        let Placed::Cell { rect, table, row, cell, story, covered: false, .. } = it else { continue };
        let (left, top, right, bottom) = (rect.x, rect.y, rect.right(), rect.bottom());
        if x < left - REACH || x > right + REACH || y < top - REACH || y > bottom + REACH {
            continue;
        }
        let along_x = x >= left && x <= right;
        let along_y = y >= top && y <= bottom;
        let sides = [
            (Side::Top, (y - top).abs(), along_x),
            (Side::Bottom, (y - bottom).abs(), along_x),
            (Side::Left, (x - left).abs(), along_y),
            (Side::Right, (x - right).abs(), along_y),
        ];
        for (side, d, along) in sides {
            let depth = table.0.len();
            let better = best.as_ref().is_none_or(|(bd, bdepth, _)| d < *bd - 0.01 || (d <= *bd + 0.01 && depth > *bdepth));
            if along && d <= REACH && better {
                best = Some((d, depth, (*story, table.clone(), *row, *cell, side, *rect)));
            }
        }
    }
    best.map(|(_, _, e)| e)
}

/// The border a Border Painter click at (`x`, `y`) on `page` would paint, end to end (page
/// points), for the app's preview.
pub fn paint_target(l: &wordcraft_layout::DocLayout, page: usize, x: f32, y: f32) -> Option<((f32, f32), (f32, f32))> {
    if !x.is_finite() || !y.is_finite() {
        return None;
    }
    let (.., side, r) = edge_at(l, page, x, y)?;
    Some(match side {
        Side::Top => ((r.x, r.y), (r.right(), r.y)),
        Side::Bottom => ((r.x, r.bottom()), (r.right(), r.bottom())),
        Side::Left => ((r.x, r.y), (r.x, r.bottom())),
        Side::Right => ((r.right(), r.y), (r.right(), r.bottom())),
    })
}

/// Sets one side of cell (`r`, `c`)'s own borders, when the cell is there.
fn set_side(t: &mut Table, r: usize, c: usize, side: Side, b: Border) {
    let Some(cell) = t.rows.get_mut(r).and_then(|row| row.cells.get_mut(c)) else { return };
    let mut own = cell.props.borders.unwrap_or_default();
    match side {
        Side::Top => own.top = Some(b),
        Side::Bottom => own.bottom = Some(b),
        Side::Left => own.left = Some(b),
        Side::Right => own.right = Some(b),
    }
    cell.props.borders = Some(own);
}

/// Whether cell (`r`, `c`) continues a vertical merge from the row above.
fn continues(t: &Table, r: usize, c: usize) -> bool {
    t.rows.get(r).and_then(|row| row.cells.get(c)).is_some_and(|x| x.props.vmerge == VMerge::Continue)
}

/// The cell of row `r` at grid column `g` and whether it continues a merge from above.
fn at_grid(t: &Table, r: usize, g: usize) -> Option<(usize, bool)> {
    t.cell_at_grid(r, g).map(|c| (c, continues(t, r, c)))
}

/// The grid columns cell (`r`, `c`) covers.
fn grid_span(t: &Table, r: usize, c: usize) -> (usize, usize) {
    let g = t.grid_col(r, c);
    let span = t.rows.get(r).and_then(|row| row.cells.get(c)).map_or(1, |x| x.span().max(1));
    (g, g.saturating_add(span))
}

/// The last row of the vertical merge starting at row `r` in grid column `g` (`r` itself when
/// the cell isn't merged down).
fn merge_end(t: &Table, r: usize, g: usize) -> usize {
    let mut last = r;
    while last + 1 < t.rows.len() && at_grid(t, last + 1, g).is_some_and(|(_, cont)| cont) {
        last += 1;
    }
    last
}

/// Paints the horizontal line below row `upper` (above row 0 when `None`) over grid columns
/// `g0..g1`, widened to whole cells on both sides: the cells above get the pen as their bottom
/// and those below as their top, as the layout draws a line from both (a vertical merge's
/// bottom from its last cell). Where a merge runs through the line there is no line to paint.
fn paint_horizontal(t: &mut Table, upper: Option<usize>, g0: usize, g1: usize, b: Border) {
    let lower = upper.map_or(0, |u| u.saturating_add(1));
    let lower = (lower < t.rows.len()).then_some(lower);
    let crosses = |t: &Table, g: usize| lower.is_some_and(|l| at_grid(t, l, g).is_some_and(|(_, cont)| cont));
    // Widen to whole cells on both sides (bounded: the range only grows, up to the grid's width).
    let (mut a, mut z) = (g0, g1.max(g0.saturating_add(1)));
    for _ in 0..=t.cols() {
        let (mut na, mut nz) = (a, z);
        for g in a..z {
            if crosses(t, g) {
                continue;
            }
            for r in [upper, lower].into_iter().flatten() {
                if let Some(c) = t.cell_at_grid(r, g) {
                    let (s0, s1) = grid_span(t, r, c);
                    na = na.min(s0);
                    nz = nz.max(s1);
                }
            }
        }
        if (na, nz) == (a, z) {
            break;
        }
        (a, z) = (na, nz);
    }
    for g in a..z {
        if crosses(t, g) {
            continue;
        }
        if let Some(u) = upper
            && let Some(c) = t.cell_at_grid(u, g)
        {
            set_side(t, u, c, Side::Bottom, b);
        }
        if let Some(l) = lower
            && let Some(c) = t.cell_at_grid(l, g)
        {
            set_side(t, l, c, Side::Top, b);
        }
    }
}

/// Paints the clicked `side` of cell (`row`, `cell`) as the layout draws it: a top or bottom
/// line along every cell it borders (a vertical merge's bottom below its last row), a left or
/// right side on every row of a vertical merge, with each row's neighbour.
fn paint_edge(t: &mut Table, row: usize, cell: usize, side: Side, b: Border) {
    let (g0, g1) = grid_span(t, row, cell);
    let last = merge_end(t, row, g0);
    match side {
        Side::Top => paint_horizontal(t, row.checked_sub(1), g0, g1, b),
        Side::Bottom => paint_horizontal(t, Some(last), g0, g1, b),
        Side::Left => paint_vertical(t, g0, row, last, b),
        Side::Right => paint_vertical(t, g1, row, last, b),
    }
}

/// The first row of the vertical merge holding row `r` in grid column `g`.
fn merge_start(t: &Table, r: usize, g: usize) -> usize {
    let mut first = r;
    while first > 0 && at_grid(t, first, g).is_some_and(|(_, cont)| cont) {
        first -= 1;
    }
    first
}

/// Paints the vertical line at grid line `gb` over rows `r0..=r1`, widened to whole vertical
/// merges on both sides: the cell left of it gets the pen as its right side and the cell right
/// of it as its left, on every row, as the layout draws sides row by row.
fn paint_vertical(t: &mut Table, gb: usize, r0: usize, r1: usize, b: Border) {
    let sides = |t: &Table, r: usize| {
        let left = gb.checked_sub(1).and_then(|g| t.cell_at_grid(r, g));
        let right = t.cell_at_grid(r, gb);
        (left, right)
    };
    let (mut a, mut z) = (r0, r1);
    for _ in 0..=t.rows.len() {
        let (mut na, mut nz) = (a, z);
        for r in a..=z {
            for g in [gb.checked_sub(1), Some(gb)].into_iter().flatten() {
                if t.cell_at_grid(r, g).is_some() {
                    na = na.min(merge_start(t, r, g));
                    nz = nz.max(merge_end(t, r, g));
                }
            }
        }
        if (na, nz) == (a, z) {
            break;
        }
        (a, z) = (na, nz);
    }
    for r in a..=z {
        match sides(t, r) {
            // One cell spans over the line: there's no line in this row.
            (Some(l), Some(rt)) if l == rt => {}
            (l, rt) => {
                if let Some(c) = l {
                    set_side(t, r, c, Side::Right, b);
                }
                if let Some(c) = rt {
                    set_side(t, r, c, Side::Left, b);
                }
            }
        }
    }
}

/// Paints the border under a click with the pen (`paint_edge`), so the line drawn there is the
/// pen's whichever cell draws it.
fn paint_at(s: &mut Session, v: &Value) -> CmdResult {
    let page =
        p::u64(v, "page").map(|n| usize::try_from(n).unwrap_or(usize::MAX)).ok_or_else(|| CmdError::Params("`page` (number) is required".into()))?;
    let (x, y) = (p::req_f32(v, "x")?, p::req_f32(v, "y")?);
    let l = s.layout();
    if l.pages.get(page).is_none() {
        return Err(CmdError::Params(format!("no page {page}")));
    }
    let (story, tp, row, cell, seen, _) = edge_at(&l, page, x, y).ok_or_else(|| CmdError::Failed("there's no table border there".into()))?;
    let b = s.border_pen.border();
    let t = s.doc.table_mut(story, &tp)?;
    // The layout draws a right-to-left table's logical start (left) on the right.
    let side = match (t.props.bidi_visual, seen) {
        (true, Side::Left) => Side::Right,
        (true, Side::Right) => Side::Left,
        (_, side) => side,
    };
    paint_edge(t, row, cell, side, b);
    table::touch_cells_in(s, story, &tp)?;
    sel_result(s)
}
