use super::cells::{cell, selected_cells, with_cells, with_table};
use super::{
    Align, Border, BorderStyle, Borders, CmdError, CmdResult, HeightRule, Path, Pos, Rgb, Selection, Session, Table, TextDirection, VAlign, Value,
    json, p, sel_result,
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

/// Table Properties: table, row, column and cell settings in one undoable step. Row, column and
/// cell settings apply to the selected rows, columns and cells, as in Word, and to the caret's
/// when nothing is selected (#4857). Without any setting it returns the caret's current values.
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
    // The selected cells (a whole table, rows, a column…), else the caret's.
    let cells = Some(selected_cells(s)).filter(|c| !c.is_empty()).unwrap_or_else(|| vec![(r, c)]);
    let mut rows: Vec<usize> = cells.iter().map(|&(r, _)| r).collect();
    rows.sort_unstable();
    rows.dedup();
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
            set_column_widths(t, &cells, cw);
        }
        if let Some(w) = width {
            t.props.width = w;
            t.props.width_pct = None;
        }
        if let Some(pct) = width_pct {
            t.props.width_pct = Some(pct);
        }
        for &r in &rows {
            let Some(row) = t.rows.get_mut(r) else { continue };
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
        }
        for &(r, c) in &cells {
            let Some(cl) = t.rows.get_mut(r).and_then(|row| row.cells.get_mut(c)) else { continue };
            if let Some(w) = cell_width {
                cl.props.width = w;
            }
            if let Some(va) = valign {
                cl.props.valign = Some(va);
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
    let kind = p::str(v, "kind").unwrap_or("all");
    let w = p::f32(v, "width").unwrap_or(0.5).clamp(0.25, 6.0);
    let color = p::str(v, "color").and_then(Rgb::parse);
    let b = Border { style: BorderStyle::Single, width: w, color, space: 0.0 };
    let none = Border { style: BorderStyle::None, width: 0.0, color: None, space: 0.0 };
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
            _ => {}
        }
        t.props.borders = Some(cur);
        for r in &mut t.rows {
            for c in &mut r.cells {
                c.props.borders = None;
            }
        }
    })
}

/// Home › Paragraph › Borders with whole cells selected (the table, rows, columns or a block of
/// cells): the border goes on the cells as one block, as in Word, not on the paragraphs in them
/// (#7874). `all` borders every cell, `outside` the block's edges, `inside` the lines between its
/// cells, `none` clears every side, and one side toggles along that edge of the block. Other kinds
/// change nothing.
pub(crate) fn cell_block_borders(s: &mut Session, range: &crate::CellRange, kind: &str, b: Border) -> CmdResult {
    let nil = Border { style: BorderStyle::None, width: 0.0, color: None, space: 0.0 };
    let b = Border { space: 0.0, ..b };
    let one_side = ["top", "left", "bottom", "right"].iter().position(|k| *k == kind);
    if one_side.is_none() && !matches!(kind, "all" | "none" | "outside" | "inside") {
        return sel_result(s);
    }
    let t = s.doc.table_mut(range.story, &range.table)?;
    let ((r0, r1), (g0, g1)) = (range.rows, range.cols);
    // Each selected cell, and which of its sides (top, left, bottom, right) are on the block's edge.
    // A row's first and last cells are on it too when the row skips grid columns (`w:gridBefore`,
    // `w:gridAfter`): the layout draws those as the table's outer edge.
    let cells: Vec<(usize, usize, [bool; 4])> = range
        .cells(t)
        .into_iter()
        .map(|(r, c)| {
            let (a, z) = super::grid_cols(t, r, c);
            let last = t.rows.get(r).map_or(0, |row| row.cells.len().saturating_sub(1));
            (r, c, [r <= r0, a <= g0 || c == 0, r >= r1, z >= g1 || c >= last])
        })
        .collect();
    // One side is taken off when every cell along that edge already shows it, as the button toggles.
    let shown = one_side.is_some_and(|i| {
        cells.iter().filter(|(_, _, edge)| edge[i]).all(|(r, c, _)| {
            let own = t.rows.get(*r).and_then(|row| row.cells.get(*c)).and_then(|cl| cl.props.borders);
            own.as_ref().and_then(|x| side(x, i)).is_some_and(|x| x.is_visible())
        })
    });
    // The sides to set: (row, cell, side, border).
    let mut writes: Vec<(usize, usize, usize, Border)> = Vec::new();
    for (r, c, edge) in cells {
        for (i, on_edge) in edge.into_iter().enumerate() {
            let new = match kind {
                "all" => Some(b),
                "none" => Some(nil),
                "outside" if on_edge => Some(b),
                "inside" if !on_edge => Some(b),
                _ if one_side == Some(i) && on_edge => Some(if shown { nil } else { b }),
                _ => None,
            };
            let Some(new) = new else { continue };
            writes.push((r, c, i, new));
            // Each cell draws its own sides, so a line on the block's edge is also the facing
            // side of the cell beyond it: that gets the same border, or No Border would leave
            // the neighbour's line showing.
            if on_edge {
                writes.extend(beside(t, r, c, i, (g0, g1)).into_iter().map(|(nr, nc)| (nr, nc, (i + 2) % 4, new)));
            }
        }
    }
    for (r, c, i, new) in writes {
        let Some(cl) = t.rows.get_mut(r).and_then(|row| row.cells.get_mut(c)) else { continue };
        let mut cur = cl.props.borders.unwrap_or_default();
        *side_mut(&mut cur, i) = Some(new);
        cl.props.borders = Some(cur);
    }
    touch_cells(s, &range.table)?;
    sel_result(s)
}

/// The cells across side `i` (top, left, bottom, right) of cell (`r`, `c`) whose facing edge lies
/// wholly on that line. Above and below, only cells within grid columns `cols` (the selected
/// block's), so a wider cell's edge beyond the block keeps its line.
fn beside(t: &Table, r: usize, c: usize, i: usize, (g0, g1): (usize, usize)) -> Vec<(usize, usize)> {
    let Some(row) = t.rows.get(r) else { return Vec::new() };
    match i {
        1 => c.checked_sub(1).map(|p| vec![(r, p)]).unwrap_or_default(),
        3 => {
            if c + 1 < row.cells.len() {
                vec![(r, c + 1)]
            } else {
                Vec::new()
            }
        }
        _ => {
            let nr = if i == 0 { r.checked_sub(1) } else { Some(r + 1) };
            let Some((nr, next)) = nr.and_then(|nr| t.rows.get(nr).map(|x| (nr, x))) else { return Vec::new() };
            let (a, z) = super::grid_cols(t, r, c);
            (0..next.cells.len())
                .filter(|&nc| {
                    let (na, nz) = super::grid_cols(t, nr, nc);
                    na <= z && nz >= a && na >= g0 && nz <= g1
                })
                .map(|nc| (nr, nc))
                .collect()
        }
    }
}

/// Side `i` of `b`: top, left, bottom, right.
fn side(b: &Borders, i: usize) -> Option<Border> {
    match i {
        0 => b.top,
        1 => b.left,
        2 => b.bottom,
        _ => b.right,
    }
}

/// Side `i` of `b` to set: top, left, bottom, right.
fn side_mut(b: &mut Borders, i: usize) -> &mut Option<Border> {
    match i {
        0 => &mut b.top,
        1 => &mut b.left,
        2 => &mut b.bottom,
        _ => &mut b.right,
    }
}
