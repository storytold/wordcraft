//! Table Design and Table Layout tabs.

use serde_json::{Value, json};
use wordcraft_doc::props::{Align, Border, BorderStyle, Borders, HeightRule, Rgb, TextDirection, VAlign};
use wordcraft_doc::{Block, Paragraph, Path, Pos, StoryRef, Table, para_block};

use super::sel_result;
use crate::{CmdError, CmdResult, CommandSpec, Selection, Session, p};

fn in_table(s: &Session) -> Option<&'static str> {
    if s.sel.focus.path.cell().is_some() { None } else { Some("the cursor isn't in a table") }
}

pub fn specs() -> Vec<CommandSpec> {
    let t = |c: CommandSpec| c.when(in_table);
    vec![
        t(CommandSpec::new("table.insertRowAbove", "Insert Above", "Table Layout › Rows & Columns", |s, _| row(s, false))),
        t(CommandSpec::new("table.insertRowBelow", "Insert Below", "Table Layout › Rows & Columns", |s, _| row(s, true))),
        t(CommandSpec::new("table.insertColumnLeft", "Insert Left", "Table Layout › Rows & Columns", |s, _| col(s, false))),
        t(CommandSpec::new("table.insertColumnRight", "Insert Right", "Table Layout › Rows & Columns", |s, _| col(s, true))),
        t(CommandSpec::new("table.deleteRow", "Delete Rows", "Table Layout › Rows & Columns › Delete", del_row)),
        t(CommandSpec::new("table.deleteColumn", "Delete Columns", "Table Layout › Rows & Columns › Delete", del_col)),
        t(CommandSpec::new("table.deleteTable", "Delete Table", "Table Layout › Rows & Columns › Delete", del_table)),
        t(CommandSpec::new("table.deleteCells", "Delete Cells", "Table Layout › Rows & Columns › Delete", |s, _| {
            let (tp, r, c) = cell(s)?;
            let story = s.sel.focus.story;
            let t = s.doc.table_mut(story, &tp)?;
            if let Some(row) = t.rows.get_mut(r)
                && row.cells.len() > 1
                && c < row.cells.len()
            {
                row.cells.remove(c);
            }
            fix_caret(s, &tp, r, 0)
        })),
        t(CommandSpec::new("table.merge", "Merge Cells", "Table Layout › Merge", merge)),
        t(CommandSpec::new("table.split", "Split Cells", "Table Layout › Merge", |s, v| {
            let n = p::u64(v, "columns").unwrap_or(2).clamp(1, 63) as usize;
            let (tp, r, c) = cell(s)?;
            s.doc.table_mut(s.sel.focus.story, &tp)?.split_cell(r, c, n);
            fix_caret(s, &tp, r, c)
        })
        .params(r#"{"columns"?: n}"#)),
        t(CommandSpec::new("table.splitTable", "Split Table", "Table Layout › Merge", split_table)),
        t(CommandSpec::new("table.style", "Table Styles", "Table Design › Table Styles", |s, v| {
            let st = p::req_str(v, "style")?;
            let id = super::table_style::table_style_id(s, st).ok_or_else(|| CmdError::Params(format!("no table style `{st}`")))?;
            with_table(s, |t| t.props.style = Some(id.clone()))
        })
        .params(r#"{"style": string}"#)),
        t(CommandSpec::new("table.look", "Table Style Options", "Table Design › Table Style Options", |s, v| {
            let set = |k: &str, cur: bool| p::bool(v, k).unwrap_or(cur);
            with_table(s, |t| {
                let l = &mut t.props.look;
                l.header_row = set("headerRow", l.header_row);
                l.total_row = set("totalRow", l.total_row);
                l.banded_rows = set("bandedRows", l.banded_rows);
                l.first_column = set("firstColumn", l.first_column);
                l.last_column = set("lastColumn", l.last_column);
                l.banded_columns = set("bandedColumns", l.banded_columns);
            })
        })
        .params(
            r#"{"headerRow"?: bool, "totalRow"?: bool, "bandedRows"?: bool, "firstColumn"?: bool, "lastColumn"?: bool, "bandedColumns"?: bool}"#,
        )),
        t(CommandSpec::new("table.shading", "Shading", "Table Design › Table Styles", |s, v| {
            let c = p::str(v, "color").and_then(Rgb::parse);
            with_cells(s, |cl| cl.props.shading = c)
        })
        .params(r#"{"color": "RRGGBB" | null}"#)),
        t(CommandSpec::new("table.borders", "Borders", "Table Design › Borders", borders)
            .params(r#"{"kind": "all|outside|inside|none|top|bottom|left|right", "width"?: pt, "color"?: "RRGGBB"}"#)),
        t(CommandSpec::new("table.cellAlign", "Alignment", "Table Layout › Alignment", |s, v| {
            let (h, va) = match p::str(v, "value").unwrap_or("topLeft") {
                "topLeft" => (Align::Left, VAlign::Top),
                "topCenter" => (Align::Center, VAlign::Top),
                "topRight" => (Align::Right, VAlign::Top),
                "centerLeft" => (Align::Left, VAlign::Center),
                "center" => (Align::Center, VAlign::Center),
                "centerRight" => (Align::Right, VAlign::Center),
                "bottomLeft" => (Align::Left, VAlign::Bottom),
                "bottomCenter" => (Align::Center, VAlign::Bottom),
                "bottomRight" => (Align::Right, VAlign::Bottom),
                x => return Err(CmdError::Params(format!("unknown alignment `{x}`"))),
            };
            with_cells(s, |cl| {
                cl.props.valign = va;
                for b in &mut cl.blocks {
                    if let Block::Para(p) = std::sync::Arc::make_mut(b) {
                        p.props.align = Some(h);
                        p.touch();
                    }
                }
            })
        })
        .params(r#"{"value": "topLeft|topCenter|…|bottomRight"}"#)),
        t(CommandSpec::new("table.textDirection", "Text Direction", "Table Layout › Alignment", text_direction)
            .params(r#"{"value"?: "horizontal|down|up"} (default: the next direction after the caret cell's, like Word's button)"#)),
        t(CommandSpec::new("table.autofit", "AutoFit", "Table Layout › Cell Size", autofit).params(r#"{"mode": "contents|window|fixed"}"#)),
        t(CommandSpec::new("table.distributeColumns", "Distribute Columns", "Table Layout › Cell Size", |s, _| {
            with_table(s, |t| {
                let total: f32 = t.grid.iter().sum();
                let n = t.grid.len().max(1) as f32;
                t.grid.iter_mut().for_each(|g| *g = total / n);
                sync_cell_widths(t);
            })
        })),
        t(CommandSpec::new("table.distributeRows", "Distribute Rows", "Table Layout › Cell Size", |s, _| {
            with_table(s, |t| {
                let h = t.rows.iter().filter_map(|r| r.props.height).fold(0.0f32, f32::max).max(18.0);
                for r in &mut t.rows {
                    r.props.height = Some(h);
                }
            })
        })),
        t(CommandSpec::new("table.columnWidth", "Table Column Width", "Table Layout › Cell Size", |s, v| {
            let w = p::req_f32(v, "width")?.clamp(6.0, 1584.0);
            let (_, r, c) = cell(s)?;
            with_table(s, |t| set_column_width(t, r, c, w))
        })
        .params(r#"{"width": pt} (the caret cell's column; the table's preferred width is cleared so the column gets exactly this)"#)),
        t(CommandSpec::new("table.rowHeight", "Table Row Height", "Table Layout › Cell Size", |s, v| {
            let h = p::req_f32(v, "height")?.clamp(1.0, 1584.0);
            let rule = match p::str(v, "rule") {
                None | Some("atLeast") => HeightRule::AtLeast,
                Some("exact") => HeightRule::Exact,
                Some(x) => return Err(CmdError::Params(format!("unknown height rule `{x}`"))),
            };
            let (_, r, _) = cell(s)?;
            with_table(s, |t| {
                if let Some(row) = t.rows.get_mut(r) {
                    row.props.height = Some(h);
                    row.props.height_rule = rule;
                }
            })
        })
        .params(r#"{"height": pt, "rule"?: "atLeast|exact"}"#)),
        t(CommandSpec::new("table.repeatHeader", "Repeat Header Rows", "Table Layout › Data", |s, v| {
            let (_, r, _) = cell(s)?;
            let on = p::bool(v, "value");
            with_table(s, |t| {
                if let Some(row) = t.rows.get_mut(r) {
                    row.props.header = on.unwrap_or(!row.props.header);
                }
            })
        })),
        t(CommandSpec::new("table.sort", "Sort", "Table Layout › Data", sort_table)
            .params(r#"{"column"?: n, "descending"?: bool, "header"?: bool}"#)),
        t(CommandSpec::new("table.toText", "Convert to Text", "Table Layout › Data", to_text).params(r#"{"separator"?: "tab|comma|paragraph"}"#)),
        t(CommandSpec::new("table.formula", "Formula", "Table Layout › Data", formula).params(r#"{"formula"?: "=SUM(ABOVE)"}"#)),
        t(CommandSpec::new("table.selectTable", "Select Table", "Table Layout › Table › Select", |s, _| {
            let (tp, _, _) = cell(s)?;
            select_cells(s, &tp, None)
        })
        .pure()),
        t(CommandSpec::new("table.selectRow", "Select Row", "Table Layout › Table › Select", |s, _| {
            let (tp, r, _) = cell(s)?;
            select_cells(s, &tp, Some((r, None)))
        })
        .pure()),
        t(CommandSpec::new("table.selectCell", "Select Cell", "Table Layout › Table › Select", |s, _| {
            let (tp, r, c) = cell(s)?;
            select_cells(s, &tp, Some((r, Some(c))))
        })
        .pure()),
        t(CommandSpec::new("table.rtl", "Right-to-Left Table", "Table Layout › Table", |s, v| {
            let (tp, _, _) = cell(s)?;
            let on = match p::bool(v, "value") {
                Some(b) => b,
                None => !s.doc.table(s.sel.focus.story, &tp).is_some_and(|t| t.props.bidi_visual),
            };
            with_table(s, |t| t.props.bidi_visual = on)
        })
        .params(r#"{"value"?: bool} (true: the first column stands at the right; without a value it toggles)"#)),
        t(CommandSpec::new("table.properties", "Properties", "Table Layout › Table", properties).params(
            r#"{"align"?: "left|center|right", "rtl"?: bool, "width"?: pt|null, "widthPct"?: percent, "indent"?: pt, "rowHeight"?: pt|null, "rowHeightRule"?: "atLeast|exact", "allowBreak"?: bool, "headerRow"?: bool, "columnWidth"?: pt, "cellWidth"?: pt|null, "valign"?: "top|center|bottom"} (row, column and cell settings apply to the caret's; without settings it returns the current ones)"#,
        )),
        CommandSpec::new("table.fromText", "Convert Text to Table", "Insert › Tables", from_text).params(r#"{"separator"?: "tab|comma"}"#),
        CommandSpec::new("table.quick", "Quick Tables", "Insert › Tables", quick_table).params(r#"{"kind"?: "calendar|tabular|matrix"}"#),
    ]
}

/// Table Layout › AutoFit: Contents sizes the columns to their text, Window spreads them across
/// the text column, Fixed keeps the widths as they are.
fn autofit(s: &mut Session, v: &Value) -> CmdResult {
    let mode = p::str(v, "mode").unwrap_or("window");
    let (tp, _, _) = cell(s)?;
    let story = s.sel.focus.story;
    let tw = super::page::sect(s).text_width();
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
        let mut g = 0usize;
        for c in &mut row.cells {
            let span = c.span();
            let w: f32 = grid.get(g..g.saturating_add(span).min(grid.len())).unwrap_or(&[]).iter().sum();
            if w > 0.0 {
                c.props.width = Some(w);
            }
            g += span;
        }
    }
}

/// Set the width of the grid column cell `c` of row `r` starts in. The table's preferred width
/// goes, so the layout doesn't scale the column back.
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
fn properties(s: &mut Session, v: &Value) -> CmdResult {
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
                    cl.props.valign = va;
                }
            }
        }
    })
}

fn cell(s: &Session) -> Result<(Path, usize, usize), CmdError> {
    s.sel.focus.path.cell().ok_or_else(|| CmdError::Disabled("the cursor isn't in a table".into()))
}

fn with_table(s: &mut Session, f: impl Fn(&mut Table)) -> CmdResult {
    let (tp, _, _) = cell(s)?;
    let t = s.doc.table_mut(s.sel.focus.story, &tp)?;
    f(t);
    touch_cells(s, &tp)?;
    sel_result(s)
}

/// Selected cells: (row, cell) pairs covered by the selection (at least the caret's cell).
fn selected_cells(s: &Session) -> Vec<(usize, usize)> {
    let (a, b) = s.sel.ordered();
    let (Some((ta, ra, ca)), Some((tb, rb, cb))) = (a.path.cell(), b.path.cell()) else { return Vec::new() };
    if ta != tb {
        return vec![(rb, cb)];
    }
    let (r0, r1) = (ra.min(rb), ra.max(rb));
    let (c0, c1) = (ca.min(cb), ca.max(cb));
    let mut v = Vec::new();
    for r in r0..=r1 {
        for c in c0..=c1 {
            v.push((r, c));
        }
    }
    v
}

fn with_cells(s: &mut Session, f: impl Fn(&mut wordcraft_doc::Cell)) -> CmdResult {
    let (tp, _, _) = cell(s)?;
    let cells = selected_cells(s);
    let t = s.doc.table_mut(s.sel.focus.story, &tp)?;
    for (r, c) in cells {
        if let Some(cl) = t.rows.get_mut(r).and_then(|row| row.cells.get_mut(c)) {
            f(cl);
        }
    }
    touch_cells(s, &tp)?;
    sel_result(s)
}

/// Table Layout › Text Direction: turn the selected cells' text. Without a value it cycles the
/// caret cell's direction (horizontal → down → up) and gives every selected cell the result.
fn text_direction(s: &mut Session, v: &Value) -> CmdResult {
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

fn fix_caret(s: &mut Session, tp: &Path, r: usize, c: usize) -> CmdResult {
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

fn row(s: &mut Session, below: bool) -> CmdResult {
    let (tp, r, c) = cell(s)?;
    let cells = selected_cells(s);
    let n = cells.iter().map(|x| x.0).collect::<std::collections::BTreeSet<_>>().len().max(1);
    let t = s.doc.table_mut(s.sel.focus.story, &tp)?;
    for _ in 0..n {
        t.insert_row(if below { r + 1 } else { r }, r);
    }
    fix_caret(s, &tp, if below { r + 1 } else { r }, c)
}

fn col(s: &mut Session, right: bool) -> CmdResult {
    let (tp, r, c) = cell(s)?;
    let t = s.doc.table_mut(s.sel.focus.story, &tp)?;
    let g = t.grid_col(r, c);
    let span = t.rows.get(r).and_then(|x| x.cells.get(c)).map(|x| x.span()).unwrap_or(1);
    t.insert_col(if right { g + span } else { g });
    // Keep the table within its width: shrink all columns proportionally.
    let total: f32 = t.grid.iter().sum();
    let before: f32 = total - t.grid.get(if right { g + span } else { g }).copied().unwrap_or(0.0);
    if before > 0.0 {
        let k = before / total;
        t.grid.iter_mut().for_each(|w| *w *= k);
    }
    fix_caret(s, &tp, r, if right { c + 1 } else { c })
}

fn del_row(s: &mut Session, _: &Value) -> CmdResult {
    let (tp, r, _) = cell(s)?;
    let rows: std::collections::BTreeSet<usize> = selected_cells(s).into_iter().map(|x| x.0).collect();
    let story = s.sel.focus.story;
    let t = s.doc.table_mut(story, &tp)?;
    if rows.len() >= t.rows.len() {
        return del_table(s, &Value::Null);
    }
    for rr in rows.iter().rev() {
        t.delete_row(*rr);
    }
    fix_caret(s, &tp, r, 0)
}

fn del_col(s: &mut Session, _: &Value) -> CmdResult {
    let (tp, r, c) = cell(s)?;
    let story = s.sel.focus.story;
    let t = s.doc.table_mut(story, &tp)?;
    if t.grid.len() <= 1 {
        return del_table(s, &Value::Null);
    }
    let g = t.grid_col(r, c);
    t.delete_col(g);
    // Rows left without cells are dropped; if that was every row, no table is left.
    if t.rows.is_empty() {
        return del_table(s, &Value::Null);
    }
    fix_caret(s, &tp, r, c.saturating_sub(1))
}

fn del_table(s: &mut Session, _: &Value) -> CmdResult {
    let (tp, _, _) = cell(s)?;
    let story = s.sel.focus.story;
    s.doc.remove_block(story, &tp)?;
    s.sel = Selection::caret(s.doc.clamp(&Pos { story, path: tp, off: 0 }));
    sel_result(s)
}

fn merge(s: &mut Session, _: &Value) -> CmdResult {
    let (tp, _, _) = cell(s)?;
    let cells = selected_cells(s);
    if cells.len() < 2 {
        return Err(CmdError::Failed("select two or more cells to merge".into()));
    }
    let story = s.sel.focus.story;
    let (r0, r1) = (cells.iter().map(|x| x.0).min().unwrap_or(0), cells.iter().map(|x| x.0).max().unwrap_or(0));
    let t = s.doc.table_mut(story, &tp)?;
    let c0 = cells.iter().map(|x| x.1).min().unwrap_or(0);
    let c1 = cells.iter().map(|x| x.1).max().unwrap_or(0);
    let g0 = t.grid_col(r0, c0);
    let g1 = t.grid_col(r0, c1) + t.rows.get(r0).and_then(|r| r.cells.get(c1)).map(|c| c.span()).unwrap_or(1) - 1;
    t.merge(r0, r1, g0, g1);
    fix_caret(s, &tp, r0, c0)
}

fn split_table(s: &mut Session, _: &Value) -> CmdResult {
    let (tp, r, _) = cell(s)?;
    if r == 0 {
        return Err(CmdError::Failed("can't split at the first row".into()));
    }
    let story = s.sel.focus.story;
    let t = s.doc.table_mut(story, &tp)?;
    let tail_rows = t.rows.split_off(r);
    let tail = Table { props: t.props.clone(), grid: t.grid.clone(), rows: tail_rows };
    let after = tp.with_last(tp.last() + 1);
    s.doc.insert_block(story, &after, Block::Para(Paragraph::new()))?;
    s.doc.insert_block(story, &after.with_last(after.last() + 1), Block::Table(tail))?;
    s.sel = Selection::caret(Pos { story, path: after, off: 0 });
    sel_result(s)
}

fn borders(s: &mut Session, v: &Value) -> CmdResult {
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

fn cell_text(c: &wordcraft_doc::Cell) -> String {
    c.blocks.iter().filter_map(|b| b.as_para().map(|p| p.plain_text())).collect::<Vec<_>>().join("\n")
}

fn sort_table(s: &mut Session, v: &Value) -> CmdResult {
    let col = p::u64(v, "column").unwrap_or(0) as usize;
    let desc = p::bool(v, "descending").unwrap_or(false);
    let header = p::bool(v, "header");
    with_table(s, |t| {
        let skip = usize::from(header.unwrap_or(t.rows.first().is_some_and(|r| r.props.header)));
        let Some(body) = t.rows.get_mut(skip..) else { return };
        body.sort_by(|a, b| {
            let ka = a.cells.get(col).map(cell_text).unwrap_or_default();
            let kb = b.cells.get(col).map(cell_text).unwrap_or_default();
            let o = match (ka.trim().replace(',', "").parse::<f64>(), kb.trim().replace(',', "").parse::<f64>()) {
                (Ok(x), Ok(y)) => x.total_cmp(&y),
                _ => ka.to_lowercase().cmp(&kb.to_lowercase()),
            };
            if desc { o.reverse() } else { o }
        });
    })
}

fn to_text(s: &mut Session, v: &Value) -> CmdResult {
    let (tp, _, _) = cell(s)?;
    let sep = match p::str(v, "separator").unwrap_or("tab") {
        "comma" => ",",
        "paragraph" => "\n",
        _ => "\t",
    };
    let story = s.sel.focus.story;
    let t = s.doc.table(story, &tp).cloned().ok_or_else(|| CmdError::Failed("no table".into()))?;
    s.doc.remove_block(story, &tp)?;
    let mut i = tp.last();
    for row in &t.rows {
        let text: Vec<String> = row.cells.iter().map(cell_text).collect();
        for line in text.join(sep).split('\n') {
            s.doc.insert_block(story, &tp.with_last(i), Block::Para(Paragraph::with_text(line, Default::default())))?;
            i += 1;
        }
    }
    s.sel = Selection::caret(Pos { story, path: tp, off: 0 });
    sel_result(s)
}

/// `=SUM(ABOVE)`, `=SUM(LEFT)`, `=AVERAGE(ABOVE)`, `=COUNT(...)`, `=MAX`, `=MIN`, `=PRODUCT`.
fn formula(s: &mut Session, v: &Value) -> CmdResult {
    let (tp, r, c) = cell(s)?;
    let f = p::str(v, "formula").unwrap_or("=SUM(ABOVE)").to_ascii_uppercase();
    let t = s.doc.table(s.sel.focus.story, &tp).cloned().ok_or_else(|| CmdError::Failed("no table".into()))?;
    let nums: Vec<f64> = if f.contains("LEFT") {
        t.rows
            .get(r)
            .map(|row| row.cells.iter().take(c).filter_map(|x| cell_text(x).trim().replace([',', '$'], "").parse().ok()).collect())
            .unwrap_or_default()
    } else {
        t.rows.iter().take(r).filter_map(|row| row.cells.get(c)).filter_map(|x| cell_text(x).trim().replace([',', '$'], "").parse().ok()).collect()
    };
    let val = if f.starts_with("=AVERAGE") {
        if nums.is_empty() { 0.0 } else { nums.iter().sum::<f64>() / nums.len() as f64 }
    } else if f.starts_with("=COUNT") {
        nums.len() as f64
    } else if f.starts_with("=MAX") {
        nums.iter().copied().fold(f64::MIN, f64::max)
    } else if f.starts_with("=MIN") {
        nums.iter().copied().fold(f64::MAX, f64::min)
    } else if f.starts_with("=PRODUCT") {
        nums.iter().product()
    } else {
        nums.iter().sum()
    };
    let text = if val.fract() == 0.0 && val.abs() < 1e15 { format!("{}", val as i64) } else { format!("{val:.2}") };
    let props = s.typing_props();
    let at = s.sel.focus.clone();
    let end = s.doc.insert_object(&at, wordcraft_doc::InlineObject::Field { instr: f.clone(), result: text.clone(), locked: false }, &props)?;
    s.sel = Selection::caret(end);
    Ok(json!({"result": text}))
}

fn select_cells(s: &mut Session, tp: &Path, which: Option<(usize, Option<usize>)>) -> CmdResult {
    let story = s.sel.focus.story;
    let Some(t) = s.doc.table(story, tp) else { return sel_result(s) };
    let (r0, r1, c0, c1) = match which {
        None => (0, t.rows.len().saturating_sub(1), 0, usize::MAX),
        Some((r, None)) => (r, r, 0, usize::MAX),
        Some((r, Some(c))) => (r, r, c, c),
    };
    let last_c = t.rows.get(r1).map(|x| x.cells.len().saturating_sub(1)).unwrap_or(0).min(c1);
    let mut a = tp.0.clone();
    a.extend([r0 as u32, c0 as u32, 0]);
    let mut b = tp.0.clone();
    b.extend([r1 as u32, last_c as u32]);
    let end_path = s.doc.para_paths(story).into_iter().rfind(|q| q.0.starts_with(&b)).unwrap_or_else(|| {
        let mut x = b.clone();
        x.push(0);
        Path(x)
    });
    let off = s.doc.para(story, &end_path).map(|x| x.len()).unwrap_or(0);
    s.sel = Selection { anchor: Pos { story, path: Path(a), off: 0 }, focus: Pos { story, path: end_path, off } };
    sel_result(s)
}

fn from_text(s: &mut Session, v: &Value) -> CmdResult {
    let sep = if p::str(v, "separator") == Some("comma") { ',' } else { '\t' };
    let (a, b) = s.sel.ordered();
    if a.path.parent() != b.path.parent() || a.story != b.story {
        return Err(CmdError::Failed("select paragraphs to convert".into()));
    }
    let lines: Vec<String> =
        (a.path.last()..=b.path.last()).filter_map(|i| s.doc.para(a.story, &a.path.with_last(i)).map(|p| p.plain_text())).collect();
    let cols = lines.iter().map(|l| l.split(sep).count()).max().unwrap_or(1).clamp(1, 63);
    let width = super::page::sect(s).text_width();
    let mut t = Table::new(lines.len().max(1), cols, width);
    for (r, l) in lines.iter().enumerate() {
        for (c, txt) in l.split(sep).enumerate() {
            if let Some(cl) = t.rows.get_mut(r).and_then(|row| row.cells.get_mut(c)) {
                cl.blocks = vec![para_block(Paragraph::with_text(txt.trim(), Default::default()))];
            }
        }
    }
    for i in (a.path.last()..=b.path.last()).rev() {
        s.doc.remove_block(a.story, &a.path.with_last(i))?;
    }
    s.doc.insert_block(a.story, &a.path, Block::Table(t))?;
    let mut first = a.path.0.clone();
    first.extend([0, 0, 0]);
    s.sel = Selection::caret(Pos { story: a.story, path: Path(first), off: 0 });
    sel_result(s)
}

fn quick_table(s: &mut Session, v: &Value) -> CmdResult {
    let kind = p::str(v, "kind").unwrap_or("tabular");
    let data: Vec<Vec<&str>> = match kind {
        "calendar" => {
            let mut rows = vec![vec!["M", "T", "W", "T", "F", "S", "S"]];
            let days = [
                "", "", "1", "2", "3", "4", "5", "6", "7", "8", "9", "10", "11", "12", "13", "14", "15", "16", "17", "18", "19", "20", "21", "22",
                "23", "24", "25", "26", "27", "28", "29", "30", "31", "", "", "",
            ];
            for ch in days.chunks(7) {
                rows.push(ch.to_vec());
            }
            rows
        }
        "matrix" => vec![
            vec!["", "Column A", "Column B", "Column C"],
            vec!["Row 1", "1", "2", "3"],
            vec!["Row 2", "4", "5", "6"],
            vec!["Row 3", "7", "8", "9"],
        ],
        _ => vec![
            vec!["Item", "Needed"],
            vec!["Books", "1"],
            vec!["Magazines", "3"],
            vec!["Notebooks", "1"],
            vec!["Paper pads", "1"],
            vec!["Pens", "3"],
            vec!["Pencils", "2"],
        ],
    };
    let cols = data.iter().map(Vec::len).max().unwrap_or(1);
    let width = super::page::sect(s).text_width();
    let mut t = Table::new(data.len(), cols, width);
    t.props.style = Some(if kind == "calendar" { "GridTable1Light" } else { "GridTable4AccentBlue" }.into());
    for (r, row) in data.iter().enumerate() {
        for (c, txt) in row.iter().enumerate() {
            if let Some(cl) = t.rows.get_mut(r).and_then(|x| x.cells.get_mut(c)) {
                cl.blocks = vec![para_block(Paragraph::with_text(txt, Default::default()))];
            }
        }
    }
    if let Some(r) = t.rows.first_mut() {
        r.props.header = true;
    }
    let story = s.sel.focus.story;
    let at = super::delete_selection(s)?;
    let new = s.doc.split_paragraph(&at)?;
    s.doc.insert_block(story, &new.path, Block::Table(t))?;
    let mut first = new.path.0.clone();
    first.extend([0, 0, 0]);
    s.sel = Selection::caret(Pos { story, path: Path(first), off: 0 });
    let _ = StoryRef::Body;
    sel_result(s)
}
