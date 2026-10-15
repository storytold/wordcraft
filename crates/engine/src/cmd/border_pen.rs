//! Table Design › Borders: the border pen (Border Styles) and the Border Painter. The pen is a
//! line style, width and colour held in the session, not the document; `table.borders` draws with
//! it when no style, width or colour is given. The app owns the painter mode (a canvas tool, like
//! Draw Table and the Eraser): each click on a border runs `table.borderPainter` with page
//! coordinates, which paints just that border with the pen (one undo step).

use serde_json::{Value, json};
use wordcraft_doc::props::{Border, BorderStyle, Rgb};
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
fn color_param(v: &Value) -> Result<Option<Option<Rgb>>, CmdError> {
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
            r#"{"page": n, "x": pt, "y": pt} (a click on a table border in page points, 0-based page: paints that one border with the pen) | {"kind"?: "all|outside|inside|none|top|bottom|left|right"} (the pen on those borders of the caret's table; default all). In the app, the button turns the painter on and off."#,
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
    let kind = p::str(v, "kind").unwrap_or("all");
    s.run("table.borders", &json!({"kind": kind}))
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
        let Placed::Cell { rect, table, row, cell, story, covered: false } = it else { continue };
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

/// Paints the border under a click with the pen: the clicked cell's side and its neighbour's
/// matching side, so the line between them is the pen's whichever cell draws it.
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
    set_side(t, row, cell, side, b);
    let g = t.grid_col(row, cell);
    let neighbour = match side {
        Side::Right => Some((row, cell.saturating_add(1), Side::Left)),
        Side::Left => cell.checked_sub(1).map(|c| (row, c, Side::Right)),
        Side::Top => row.checked_sub(1).and_then(|r| t.cell_at_grid(r, g).map(|c| (r, c, Side::Bottom))),
        Side::Bottom => {
            let r = row.saturating_add(1);
            t.cell_at_grid(r, g).map(|c| (r, c, Side::Top))
        }
    };
    if let Some((r, c, other)) = neighbour {
        set_side(t, r, c, other, b);
    }
    table::touch_cells(s, &tp)?;
    sel_result(s)
}
