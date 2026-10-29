use super::cells::{cell, selected_cells, with_cells, with_table};
use super::{
    Align, BORDER_KINDS, Border, BorderStyle, Borders, CmdError, CmdResult, HashSet, HeightRule, Path, Pos, Selection, Session, Table, TextDirection,
    VAlign, VMerge, Value, border_color, border_style, border_width, json, opt_num, opt_str, p, sel_result,
};

/// Table Layout › AutoFit: Contents sizes the columns to their text, Window spreads them across
/// the text column, Fixed keeps the widths as they are.
pub(super) fn autofit(s: &mut Session, v: &Value) -> CmdResult {
    let mode = p::str(v, "mode").unwrap_or("window");
    let (tp, _, _) = cell(s)?;
    let story = s.sel.focus.story;
    let tw = crate::cmd::page::sect(s).text_width();
    match mode {
        "fixed" => with_table(s, |t| t.props.fixed = true),
        "contents" => {
            let t = s.doc.table(story, &tp).ok_or_else(|| CmdError::Disabled("the cursor isn't in a table".into()))?;
            let avail = (tw - t.props.indent.filter(|i| i.is_finite()).unwrap_or(0.0).max(0.0)).max(72.0);
            let widths = wordcraft_layout::autofit_widths(&wordcraft_layout::measure_table_columns(&s.doc, t), avail);
            with_table(s, |t| {
                t.props.fixed = false;
                t.props.width = None;
                t.props.width_pct = None;
                t.grid = widths.iter().map(|w| w.max(6.0)).collect();
                sync_cell_widths(t);
            })
        }
        _ => with_table(s, |t| {
            t.props.fixed = false;
            t.props.width_pct = Some(100.0);
            let n = t.grid.len().max(1) as f32;
            t.grid.iter_mut().for_each(|g| *g = tw / n);
            sync_cell_widths(t);
        }),
    }
}

/// Give every cell the width of the grid columns it spans, so the cells' preferred widths
/// (saved to .docx) agree with the grid.
pub(crate) fn sync_cell_widths(t: &mut Table) {
    let grid = t.grid.clone();
    for row in &mut t.rows {
        // A row's cells start after the grid columns it skips (`w:gridBefore`).
        let mut g = row.first_grid();
        for c in &mut row.cells {
            let span = c.span();
            let w: f32 = grid.get(g..g.saturating_add(span).min(grid.len())).unwrap_or(&[]).iter().sum();
            if w > 0.0 {
                c.props.width = Some(w);
            }
            g = g.saturating_add(span);
        }
    }
}

/// Set the width of the grid column cell `c` of row `r` starts in. The table's preferred width
/// goes, so the layout doesn't scale the column back.
/// Give every grid column the `cells` (row, cell) pairs touch the width `w`.
pub(super) fn set_column_widths(t: &mut Table, cells: &[(usize, usize)], w: f32) {
    let mut cols: Vec<usize> = cells.iter().map(|&(r, c)| t.grid_col(r, c)).collect();
    cols.sort_unstable();
    cols.dedup();
    let mut any = false;
    for g in cols {
        if let Some(x) = t.grid.get_mut(g) {
            *x = w;
            any = true;
        }
    }
    if any {
        t.props.width = None;
        t.props.width_pct = None;
        sync_cell_widths(t);
    }
}

fn set_column_width(t: &mut Table, r: usize, c: usize, w: f32) {
    let g = t.grid_col(r, c);
    if let Some(x) = t.grid.get_mut(g) {
        *x = w;
        t.props.width = None;
        t.props.width_pct = None;
        sync_cell_widths(t);
    }
}

/// Table Properties: table, row, column and cell settings in one undoable step. Row, column and
/// cell settings apply to the caret's. Without any setting it returns the current values.
pub(super) fn properties(s: &mut Session, v: &Value) -> CmdResult {
    let (tp, r, c) = cell(s)?;
    let keys = [
        "align",
        "rtl",
        "width",
        "widthPct",
        "indent",
        "rowHeight",
        "rowHeightRule",
        "allowBreak",
        "headerRow",
        "columnWidth",
        "cellWidth",
        "valign",
    ];
    if !keys.iter().any(|k| v.get(*k).is_some()) {
        let t = s.doc.table(s.sel.focus.story, &tp).cloned().unwrap_or_default();
        let mut out = serde_json::to_value(&t.props).unwrap_or(Value::Null);
        let row = t.rows.get(r);
        let cl = row.and_then(|x| x.cells.get(c));
        if let Some(o) = out.as_object_mut() {
            o.insert("row".into(), row.map(|x| serde_json::to_value(&x.props).unwrap_or(Value::Null)).unwrap_or(Value::Null));
            o.insert("columnWidth".into(), t.grid.get(t.grid_col(r, c)).map_or(Value::Null, |w| json!(w)));
            o.insert("cell".into(), cl.map(|x| serde_json::to_value(&x.props).unwrap_or(Value::Null)).unwrap_or(Value::Null));
        }
        return Ok(out);
    }
    let align = match p::str(v, "align") {
        None => None,
        Some("left") => Some(Align::Left),
        Some("center") => Some(Align::Center),
        Some("right") => Some(Align::Right),
        Some(x) => return Err(CmdError::Params(format!("unknown table alignment `{x}`"))),
    };
    let rule = match p::str(v, "rowHeightRule") {
        None => None,
        Some("atLeast") => Some(HeightRule::AtLeast),
        Some("exact") => Some(HeightRule::Exact),
        Some(x) => return Err(CmdError::Params(format!("unknown height rule `{x}`"))),
    };
    let valign = match p::str(v, "valign") {
        None => None,
        Some("top") => Some(VAlign::Top),
        Some("center") => Some(VAlign::Center),
        Some("bottom") => Some(VAlign::Bottom),
        Some(x) => return Err(CmdError::Params(format!("unknown vertical alignment `{x}`"))),
    };
    // `null` clears a preferred size (automatic); a number sets it.
    let size = |k: &str, lo: f32, hi: f32| -> Option<Option<f32>> {
        match v.get(k)? {
            Value::Null => Some(None),
            x => x.as_f64().map(|f| f as f32).filter(|f| f.is_finite()).map(|f| Some(f.clamp(lo, hi))),
        }
    };
    let width = size("width", 6.0, 1584.0);
    let width_pct = p::f32(v, "widthPct").map(|x| x.clamp(1.0, 100.0));
    let indent = p::f32(v, "indent").map(|x| x.clamp(-1584.0, 1584.0));
    let row_height = size("rowHeight", 1.0, 1584.0);
    let allow_break = p::bool(v, "allowBreak");
    let header = p::bool(v, "headerRow");
    let column_width = p::f32(v, "columnWidth").map(|x| x.clamp(6.0, 1584.0));
    let cell_width = size("cellWidth", 6.0, 1584.0);
    let rtl = p::bool(v, "rtl");
    with_table(s, |t| {
        if let Some(a) = align {
            t.props.align = Some(a);
        }
        if let Some(on) = rtl {
            t.props.bidi_visual = on;
        }
        if let Some(i) = indent {
            t.props.indent = Some(i);
        }
        if let Some(cw) = column_width {
            set_column_width(t, r, c, cw);
        }
        if let Some(w) = width {
            t.props.width = w;
            t.props.width_pct = None;
        }
        if let Some(pct) = width_pct {
            t.props.width_pct = Some(pct);
        }
        if let Some(row) = t.rows.get_mut(r) {
            if let Some(h) = row_height {
                row.props.height = h;
                row.props.height_rule = if h.is_some() { rule.unwrap_or(HeightRule::AtLeast) } else { HeightRule::Auto };
            } else if let Some(rule) = rule
                && row.props.height.is_some()
            {
                row.props.height_rule = rule;
            }
            if let Some(b) = allow_break {
                row.props.cant_split = !b;
            }
            if let Some(h) = header {
                row.props.header = h;
            }
            if let Some(cl) = row.cells.get_mut(c) {
                if let Some(w) = cell_width {
                    cl.props.width = w;
                }
                if let Some(va) = valign {
                    cl.props.valign = Some(va);
                }
            }
        }
    })
}

/// Table Layout › Text Direction: turn the selected cells' text. Without a value it cycles the
/// caret cell's direction (horizontal → down → up) and gives every selected cell the result.
pub(crate) fn text_direction(s: &mut Session, v: &Value) -> CmdResult {
    let (tp, r, c) = cell(s)?;
    let dir = match p::str(v, "value") {
        Some("horizontal" | "lrTb") => TextDirection::Horizontal,
        Some("down" | "tbRl") => TextDirection::Down,
        Some("up" | "btLr") => TextDirection::Up,
        Some(x) => return Err(CmdError::Params(format!("unknown text direction `{x}`"))),
        None => {
            let t = s.doc.table(s.sel.focus.story, &tp).ok_or_else(|| CmdError::Disabled("the cursor isn't in a table".into()))?;
            t.rows.get(r).and_then(|row| row.cells.get(c)).map(|cl| cl.props.text_direction).unwrap_or_default().next()
        }
    };
    with_cells(s, |cl| cl.props.text_direction = dir)
}

/// Bump the revision of every paragraph in a table (table style changes alter their layout).
pub(crate) fn touch_cells(s: &mut Session, tp: &Path) -> Result<(), CmdError> {
    let story = s.sel.focus.story;
    let paths: Vec<Path> = s.doc.para_paths(story).into_iter().filter(|p| p.0.len() > tp.0.len() && p.0.starts_with(&tp.0)).collect();
    for p in paths {
        s.doc.para_mut(story, &p)?.touch();
    }
    Ok(())
}

pub(super) fn fix_caret(s: &mut Session, tp: &Path, r: usize, c: usize) -> CmdResult {
    let story = s.sel.focus.story;
    let Some(t) = s.doc.table(story, tp) else {
        s.sel = Selection::caret(s.doc.clamp(&Pos { story, path: tp.clone(), off: 0 }));
        return sel_result(s);
    };
    let r = r.min(t.rows.len().saturating_sub(1));
    let c = c.min(t.rows.get(r).map(|x| x.cells.len().saturating_sub(1)).unwrap_or(0));
    let mut path = tp.0.clone();
    path.extend([r as u32, c as u32, 0]);
    s.sel = Selection::caret(Pos { story, path: Path(path), off: 0 });
    sel_result(s)
}

pub(super) fn borders(s: &mut Session, v: &Value) -> CmdResult {
    let kind = opt_str(v, "kind")?.unwrap_or("all");
    if !BORDER_KINDS.contains(&kind) {
        return Err(CmdError::Params(format!("unknown border kind `{kind}`")));
    }
    let pen = s.table_pen;
    let style = match opt_str(v, "style")? {
        Some(name) => border_style(name)?,
        None => pen.style,
    };
    let w = opt_num(v, "width")?.map_or(pen.width, border_width);
    let color = border_color(v)?.unwrap_or(pen.color);
    let none = Border { style: BorderStyle::None, width: 0.0, color: None, space: 0.0 };
    let b = if style == BorderStyle::None { none } else { Border { style, width: w, color, space: 0.0 } };
    let apply_to = opt_str(v, "applyTo")?;
    let (tp, _, _) = cell(s)?;
    let cells = selected_cells(s);
    let whole = match apply_to {
        Some("table") => true,
        Some("cells") => false,
        None => s.doc.table(s.sel.focus.story, &tp).is_some_and(|t| covers_table(t, &cells)),
        Some(x) => return Err(CmdError::Params(format!("unknown applyTo `{x}`"))),
    };
    if !whole {
        if cells.is_empty() {
            return Err(CmdError::Failed("no table cells are selected".into()));
        }
        return with_table(s, |t| cell_borders(t, &cells, kind, Some(b)));
    }
    with_table(s, |t| {
        let mut cur = t.props.borders.unwrap_or_default();
        match kind {
            "all" => cur = Borders::all(b),
            "none" => cur = Borders::all(none),
            "outside" => {
                cur.top = Some(b);
                cur.bottom = Some(b);
                cur.left = Some(b);
                cur.right = Some(b);
                // The cleared inside sides must not fall back to the table style (Word writes nil).
                cur.between = Some(none);
                cur.inside_v = Some(none);
            }
            "inside" => {
                cur.between = Some(b);
                cur.inside_v = Some(b);
                // The cleared outer sides must not fall back to the table style (Word writes nil).
                cur.top = Some(none);
                cur.bottom = Some(none);
                cur.left = Some(none);
                cur.right = Some(none);
            }
            "top" => cur.top = Some(b),
            "bottom" => cur.bottom = Some(b),
            "left" => cur.left = Some(b),
            "right" => cur.right = Some(b),
            "insideH" => cur.between = Some(b),
            "insideV" => cur.inside_v = Some(b),
            _ => {}
        }
        t.props.borders = Some(cur);
        // The cells' own lines on the sides the kind covers give way to the table's; the rest
        // (a double line under the header row, say) stay.
        let all: Vec<(usize, usize)> = t.rows.iter().enumerate().flat_map(|(r, row)| (0..row.cells.len()).map(move |c| (r, c))).collect();
        cell_borders(t, &all, kind, None);
    })
}

/// Whether `cells` are every cell of `t`.
fn covers_table(t: &Table, cells: &[(usize, usize)]) -> bool {
    let n: usize = t.rows.iter().map(|r| r.cells.len()).sum();
    let mut seen: Vec<_> = cells.iter().filter(|(r, c)| t.rows.get(*r).is_some_and(|row| *c < row.cells.len())).collect();
    seen.sort_unstable();
    seen.dedup();
    seen.len() == n
}

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
enum Side {
    Top,
    Bottom,
    Left,
    Right,
}

impl Side {
    fn facing(self) -> Side {
        match self {
            Side::Top => Side::Bottom,
            Side::Bottom => Side::Top,
            Side::Left => Side::Right,
            Side::Right => Side::Left,
        }
    }
}

/// Lines on the selected cells (row, cell index): the stretches of the selection's edges `kind`
/// names, outer (`top`, `bottom`, `left`, `right`, `outside`) or between selected cells
/// (`inside`, `insideH`, `insideV`); `all` and `none` every one. `line` is the border to set, or
/// `None` to take the cells' own away there. A cell merged down is selected whole. Where a side
/// faces cells of both kinds (a merged cell partly selected) only the cells across get the line;
/// the cell across an outer side gets the facing side too, also when it is wider than one
/// selected cell but every cell along it has the line, so the shared line is one border.
fn cell_borders(t: &mut Table, cells: &[(usize, usize)], kind: &str, line: Option<Border>) {
    // Every cell's grid columns, by row.
    let ranges: Vec<Vec<(usize, usize)>> = t
        .rows
        .iter()
        .map(|row| {
            let mut at = 0usize;
            row.cells
                .iter()
                .map(|c| {
                    let r = (at, at.saturating_add(c.span()));
                    at = r.1;
                    r
                })
                .collect()
        })
        .collect();
    let range = |r: usize, c: usize| ranges.get(r).and_then(|row| row.get(c)).copied();
    let at_grid = |r: usize, g: usize| ranges.get(r).and_then(|row| row.iter().position(|&(a, _)| a == g));
    let vmerge = |r: usize, c: usize| t.rows.get(r).and_then(|row| row.cells.get(c)).map(|x| x.props.vmerge);
    let mut selected: HashSet<(usize, usize)> = cells.iter().copied().filter(|&(r, c)| range(r, c).is_some()).collect();
    // A vertical merge: from its first cell down through the cells continuing it.
    let mut order: Vec<(usize, usize)> = selected.iter().copied().collect();
    order.sort_unstable();
    let mut chained: HashSet<(usize, usize)> = HashSet::new();
    for &(r, c) in &order {
        let Some((g, _)) = range(r, c) else { continue };
        if vmerge(r, c).is_none_or(|v| v == VMerge::None) || chained.contains(&(r, c)) {
            continue;
        }
        let mut first = (r, c);
        while vmerge(first.0, first.1) == Some(VMerge::Continue)
            && let Some(up) = first.0.checked_sub(1)
            && let Some(cu) = at_grid(up, g)
        {
            first = (up, cu);
        }
        let mut at = Some(first);
        while let Some((rr, cc)) = at {
            chained.insert((rr, cc));
            at = at_grid(rr + 1, g).filter(|&cn| vmerge(rr + 1, cn) == Some(VMerge::Continue)).map(|cn| (rr + 1, cn));
        }
    }
    selected.extend(chained);
    let mut order: Vec<(usize, usize)> = selected.iter().copied().collect();
    order.sort_unstable();
    // The cells across `side` of (r, c), each with whether it lies within (r, c)'s columns.
    let across = |r: usize, c: usize, side: Side| -> Vec<(usize, usize, bool)> {
        let Some((g0, g1)) = range(r, c) else { return Vec::new() };
        let row_across = match side {
            Side::Top => r.checked_sub(1),
            Side::Bottom => Some(r + 1),
            Side::Left => return c.checked_sub(1).map(|cl| vec![(r, cl, true)]).unwrap_or_default(),
            Side::Right => return range(r, c + 1).map(|_| vec![(r, c + 1, true)]).unwrap_or_default(),
        };
        let Some(ra) = row_across else { return Vec::new() };
        ranges
            .get(ra)
            .map(|row| row.iter().enumerate().filter(|&(_, &(a, z))| z > g0 && a < g1).map(|(ca, &(a, z))| (ra, ca, a >= g0 && z <= g1)).collect())
            .unwrap_or_default()
    };
    let wants = |side: Side| -> (bool, bool) {
        let (outer_kind, inner_kind) = match side {
            Side::Top => ("top", "insideH"),
            Side::Bottom => ("bottom", "insideH"),
            Side::Left => ("left", "insideV"),
            Side::Right => ("right", "insideV"),
        };
        match kind {
            "all" | "none" => (true, true),
            "outside" => (true, false),
            "inside" => (false, true),
            k => (k == outer_kind, k == inner_kind),
        }
    };
    let mut own: HashSet<(usize, usize, Side)> = HashSet::new();
    let mut writes: Vec<(usize, usize, Side)> = Vec::new();
    // Unselected cells across an outer side that are wider than the selected cell: (row, cell,
    // facing side, the selected cell's row).
    let mut wide: Vec<(usize, usize, Side, usize)> = Vec::new();
    for &(r, c) in &order {
        for side in [Side::Top, Side::Bottom, Side::Left, Side::Right] {
            let (want_outer, want_inner) = wants(side);
            if !want_outer && !want_inner {
                continue;
            }
            let others = across(r, c, side);
            let is_selected = |n: &(usize, usize, bool)| selected.contains(&(n.0, n.1));
            let outer = others.iter().all(|n| !is_selected(n));
            let inner = !others.is_empty() && others.iter().all(is_selected);
            if (outer && want_outer) || (inner && want_inner) {
                own.insert((r, c, side));
                writes.push((r, c, side));
                for n in &others {
                    if n.2 {
                        writes.push((n.0, n.1, side.facing()));
                    } else if !is_selected(n) {
                        wide.push((n.0, n.1, side.facing(), r));
                    }
                }
            } else if !outer && !inner {
                for n in others.iter().filter(|n| n.2 && if is_selected(n) { want_inner } else { want_outer }) {
                    writes.push((n.0, n.1, side.facing()));
                }
            }
        }
    }
    for (nr, nc, nside, row) in wide {
        let Some((a, z)) = range(nr, nc) else { continue };
        let side = nside.facing();
        let along =
            ranges.get(row).map(|cs| cs.iter().enumerate().filter(|&(_, &(x0, x1))| x1 > a && x0 < z).all(|(cc, _)| own.contains(&(row, cc, side))));
        if along == Some(true) {
            writes.push((nr, nc, nside));
        }
    }
    for (r, c, side) in writes {
        let Some(cell) = t.rows.get_mut(r).and_then(|row| row.cells.get_mut(c)) else { continue };
        if line.is_none() && cell.props.borders.is_none() {
            continue;
        }
        let b = cell.props.borders.get_or_insert_with(Borders::default);
        let slot = match side {
            Side::Top => &mut b.top,
            Side::Bottom => &mut b.bottom,
            Side::Left => &mut b.left,
            Side::Right => &mut b.right,
        };
        *slot = line;
        if *b == Borders::default() {
            cell.props.borders = None;
        }
    }
}
