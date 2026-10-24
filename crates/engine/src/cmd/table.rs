//! Table Design and Table Layout tabs.

use super::table_formula::parse_number;
use serde_json::{Value, json};
use wordcraft_doc::collate::{SortKey, sort_key};
use wordcraft_doc::props::{Align, Border, BorderStyle, Borders, HeightRule, Rgb, TextDirection, VAlign};
use wordcraft_doc::{Block, Document, Paragraph, Path, Pos, RevisionKind, StoryRef, Table, para_block};

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
            // A row's last cell stays.
            if !s.doc.table(story, &tp).and_then(|t| t.rows.get(r)).is_some_and(|row| row.cells.len() > 1 && c < row.cells.len()) {
                return fix_caret(s, &tp, r, 0);
            }
            let spans = text_spans(s, story, &[cell_prefix(&tp, r, c)]);
            if let Some(done) = guard_delete(s, &spans)? {
                return Ok(done);
            }
            if let Some(row) = s.doc.table_mut(story, &tp)?.rows.get_mut(r)
                && c < row.cells.len()
            {
                row.cells.remove(c);
            }
            fix_caret(s, &tp, r, 0)
        })),
        t(CommandSpec::new("table.merge", "Merge Cells", "Table Layout › Merge", merge)),
        t(CommandSpec::new("table.split", "Split Cells", "Table Layout › Merge", split)
            .params(r#"{"columns"?: n (2), "rows"?: n (1), "mergeFirst"?: bool (true: several selected cells are merged first)}"#)),
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
                cl.props.valign = Some(va);
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
            let cells = selected_cells(s);
            with_table(s, |t| set_column_widths(t, &cells, w))
        })
        .params(
            r#"{"width": pt} (every column the selection covers, the caret cell's without one; the table's preferred width is cleared so the columns get exactly this)"#,
        )),
        t(CommandSpec::new("table.rowHeight", "Table Row Height", "Table Layout › Cell Size", |s, v| {
            let h = p::req_f32(v, "height")?.clamp(1.0, 1584.0);
            let rule = match p::str(v, "rule") {
                None | Some("atLeast") => HeightRule::AtLeast,
                Some("exact") => HeightRule::Exact,
                Some(x) => return Err(CmdError::Params(format!("unknown height rule `{x}`"))),
            };
            let mut rows: Vec<usize> = selected_cells(s).into_iter().map(|(r, _)| r).collect();
            rows.dedup();
            with_table(s, |t| {
                for r in &rows {
                    if let Some(row) = t.rows.get_mut(*r) {
                        row.props.height = Some(h);
                        row.props.height_rule = rule;
                    }
                }
            })
        })
        .params(r#"{"height": pt, "rule"?: "atLeast|exact"} (every row the selection covers, the caret's without one)"#)),
        t(CommandSpec::new("table.repeatHeader", "Repeat Header Rows", "Table Layout › Data", |s, v| {
            let (_, r, _) = cell(s)?;
            let on = p::bool(v, "value");
            with_table(s, |t| {
                if let Some(row) = t.rows.get_mut(r) {
                    row.props.header = on.unwrap_or(!row.props.header);
                }
            })
        })),
        // Table Properties › Alt Text: the title (`w:tblCaption`) and description (`w:tblDescription`).
        t(CommandSpec::new("table.altText", "Alt Text", "Table Layout › Table › Properties", |s, v| {
            let (title, text) = (p::str(v, "title"), p::str(v, "text"));
            if title.is_none() && text.is_none() {
                return Err(CmdError::Params("`text` or `title` (string) is required".into()));
            }
            let set = |x: Option<&str>, cur: &Option<String>| match x {
                Some(x) if x.trim().is_empty() => None,
                Some(x) => Some(wordcraft_doc::para::cap_alt(x)),
                None => cur.clone(),
            };
            with_table(s, |t| {
                t.props.caption = set(title, &t.props.caption);
                t.props.description = set(text, &t.props.description);
            })
        })
        .params(r#"{"text"?: string, "title"?: string} (empty clears)"#)),
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
        t(CommandSpec::new("table.selectColumn", "Select Column", "Table Layout › Table › Select", select_column).pure()),
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
        CommandSpec::new("table.fromText", "Convert Text to Table", "Insert › Tables", from_text)
            .params(r#"{"separator"?: "tab|comma|paragraph|other", "char"?: string (with other), "columns"?: n, "autofit"?: "initial|fixed|contents|window", "width"?: pt (initial column width, 0 = Auto, at most 1584)}"#),
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
/// Give every grid column the `cells` (row, cell) pairs touch the width `w`.
fn set_column_widths(t: &mut Table, cells: &[(usize, usize)], w: f32) {
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
                    cl.props.valign = Some(va);
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
pub fn selected_cells(s: &Session) -> Vec<(usize, usize)> {
    // Whole cells selected (a column, say): just those, by grid column, so merged cells count.
    if let Some(range) = s.cell_range()
        && s.sel.focus.path.cell().is_some_and(|(tp, _, _)| tp == range.table)
        && let Some(t) = s.doc.table(range.story, &range.table)
    {
        return range.cells(t);
    }
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

/// Insert Above / Below: rows like the caret's, as many as the selection spans; the caret goes into the first.
pub(super) fn row(s: &mut Session, below: bool) -> CmdResult {
    let (tp, r, c) = cell(s)?;
    let cells = selected_cells(s);
    let n = cells.iter().map(|x| x.0).collect::<std::collections::BTreeSet<_>>().len().max(1);
    let t = s.doc.table_mut(s.sel.focus.story, &tp)?;
    for _ in 0..n {
        t.insert_row(if below { r + 1 } else { r }, r);
    }
    fix_caret(s, &tp, if below { r + 1 } else { r }, c)
}

/// Insert Left / Right: a column beside the caret's cell; the caret goes into it.
pub(super) fn col(s: &mut Session, right: bool) -> CmdResult {
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
    if rows.len() >= s.doc.table(story, &tp).map_or(0, |t| t.rows.len()) {
        return del_table(s, &Value::Null);
    }
    let prefixes: Vec<Vec<u32>> = rows.iter().map(|rr| tp.0.iter().copied().chain([*rr as u32]).collect()).collect();
    let spans = text_spans(s, story, &prefixes);
    if let Some(done) = guard_delete(s, &spans)? {
        return Ok(done);
    }
    let t = s.doc.table_mut(story, &tp)?;
    for rr in rows.iter().rev() {
        t.delete_row(*rr);
    }
    fix_caret(s, &tp, r, 0)
}

fn del_col(s: &mut Session, _: &Value) -> CmdResult {
    let (tp, r, c) = cell(s)?;
    let cells = selected_cells(s);
    let story = s.sel.focus.story;
    let Some(t) = s.doc.table(story, &tp) else { return Err(CmdError::Failed("no table".into())) };
    if t.grid.len() <= 1 {
        return del_table(s, &Value::Null);
    }
    // Every grid column a selected cell covers goes (the caret's alone without a selection).
    let mut cols = std::collections::BTreeSet::new();
    for (rr, cc) in cells {
        let g = t.grid_col(rr, cc);
        let span = t.rows.get(rr).and_then(|x| x.cells.get(cc)).map_or(1, wordcraft_doc::Cell::span);
        cols.extend(g..g.saturating_add(span).min(t.grid.len()));
    }
    if cols.is_empty() {
        cols.insert(t.grid_col(r, c));
    }
    if cols.len() >= t.grid.len() {
        return del_table(s, &Value::Null);
    }
    // The cells that go with those columns, and their text: those all of whose columns go
    // (a merged cell that also spans a kept column just narrows, keeping its text).
    let mut prefixes = Vec::new();
    for (rr, row) in t.rows.iter().enumerate() {
        for (cc, x) in row.cells.iter().enumerate() {
            let g = t.grid_col(rr, cc);
            let covered = g..g.saturating_add(x.span()).min(t.grid.len());
            if !covered.is_empty() && covered.clone().all(|k| cols.contains(&k)) {
                prefixes.push(cell_prefix(&tp, rr, cc));
            }
        }
    }
    let spans = text_spans(s, story, &prefixes);
    if let Some(done) = guard_delete(s, &spans)? {
        return Ok(done);
    }
    let t = s.doc.table_mut(story, &tp)?;
    for g in cols.iter().rev() {
        t.delete_col(*g);
    }
    // Rows left without cells are dropped; if that was every row, no table is left.
    if t.rows.is_empty() {
        return del_table(s, &Value::Null);
    }
    fix_caret(s, &tp, r, c.saturating_sub(1))
}

/// Backspace with whole cells selected (#1844): whole rows go like Delete Rows (every row: the
/// table, like Delete Table), whole columns like Delete Columns, with their undo and Track
/// Changes. `None` for any other block of cells, and for a single cell of a table with more
/// (Backspace then clears them).
pub(crate) fn backspace_cells(s: &mut Session) -> Option<CmdResult> {
    let cr = s.cell_range()?.clone();
    let (tp, _, _) = s.sel.focus.path.cell()?;
    if cr.story != s.sel.focus.story || cr.table != tp {
        return None;
    }
    let t = s.doc.table(cr.story, &tp)?;
    // One cell of several is just that cell, also in a one-row or one-column table (Word's cell
    // selection doesn't take the end-of-row mark): Backspace clears it.
    let cells: usize = t.rows.iter().map(|row| row.cells.len()).sum();
    if cells > 1 && cr.cells(t).len() <= 1 {
        return None;
    }
    let (last_row, last_col) = (t.rows.len().saturating_sub(1), t.cols().saturating_sub(1));
    let r = if cr.cols == (0, last_col) {
        del_row(s, &Value::Null)
    } else if cr.rows == (0, last_row) {
        del_col(s, &Value::Null)
    } else {
        return None;
    };
    s.column = None;
    Some(r)
}

/// Delete with whole cells selected (#2802): each cell is emptied to one empty paragraph, as in
/// Word, nested tables included. That paragraph keeps the formatting of the cell's last one (its
/// mark is the end-of-cell mark). `None` without whole cells, and with Track Changes on (the text
/// is marked deleted as for any selection).
pub(crate) fn clear_cells(s: &mut Session) -> Option<CmdResult> {
    if s.doc.settings.track_changes {
        return None;
    }
    let cr = s.cell_range()?.clone();
    let t = s.doc.table(cr.story, &cr.table)?;
    let prefixes: Vec<Vec<u32>> = cr.cells(t).into_iter().map(|(r, c)| cell_prefix(&cr.table, r, c)).collect();
    let spans = text_spans(s, cr.story, &prefixes);
    Some(clear_spans(s, cr.table.0.len().saturating_add(2), &spans))
}

/// Empty each span (one cell's text, from its first paragraph to its last; the cell's path is the
/// first `depth` steps of its paths) to one paragraph with the last paragraph's formatting, bottom
/// up so earlier spans stay valid; the caret goes to the start of the first cell.
fn clear_spans(s: &mut Session, depth: usize, spans: &[(Pos, Pos)]) -> CmdResult {
    // A cell's first block (its first paragraph may have been in a nested table that goes).
    let first_block = |p: &Pos| {
        let mut v = p.path.0.get(..depth).unwrap_or_default().to_vec();
        v.push(0);
        Pos { story: p.story, path: Path(v), off: 0 }
    };
    for (a, b) in spans {
        super::controls::check_delete(s, a, b)?;
    }
    for (a, b) in spans.iter().rev() {
        let Some(last) = s.doc.para(b.story, &b.path).map(|p| (p.props.clone(), p.mark.clone())) else { continue };
        s.doc.delete_range(a, b)?;
        // Now the cell's only block, unless it holds a table nested deeper than the paragraph
        // walk (`delete_range` leaves those).
        let first = first_block(a);
        if s.doc.container(first.story, &first.path).is_some_and(|bl| bl.len() == 1)
            && let Ok(p) = s.doc.para_mut(first.story, &first.path)
        {
            (p.props, p.mark) = last;
            p.touch();
        }
    }
    let at = spans.first().map(|(a, _)| s.doc.clamp(&first_block(a))).unwrap_or_else(|| s.sel.focus.clone());
    s.sel = Selection::caret(at);
    s.column = None;
    s.column_mode = false;
    s.goal_x = None;
    s.pending = None;
    super::controls::restore_placeholder(s)?;
    sel_result(s)
}

fn del_table(s: &mut Session, _: &Value) -> CmdResult {
    let (tp, _, _) = cell(s)?;
    let story = s.sel.focus.story;
    let spans = text_spans(s, story, std::slice::from_ref(&tp.0));
    if let Some(done) = guard_delete(s, &spans)? {
        return Ok(done);
    }
    s.doc.remove_block(story, &tp)?;
    s.sel = Selection::caret(s.doc.clamp(&Pos { story, path: tp, off: 0 }));
    sel_result(s)
}

/// The path prefix of cell (`r`, `c`) of the table at `tp`.
fn cell_prefix(tp: &Path, r: usize, c: usize) -> Vec<u32> {
    tp.0.iter().copied().chain([r as u32, c as u32]).collect()
}

/// The text under each path prefix (a cell, a row or a whole table): from its first paragraph's
/// start to its last paragraph's end. Prefixes without paragraphs are left out.
fn text_spans(s: &Session, story: StoryRef, prefixes: &[Vec<u32>]) -> Vec<(Pos, Pos)> {
    let paths = s.doc.para_paths(story);
    prefixes
        .iter()
        .filter_map(|pre| {
            let mut inside = paths.iter().filter(|p| p.0.starts_with(pre));
            let first = inside.next()?.clone();
            let last = inside.next_back().cloned().unwrap_or_else(|| first.clone());
            let end = s.doc.para(story, &last).map_or(0, Paragraph::len);
            Some((Pos { story, path: first, off: 0 }, Pos { story, path: last, off: end }))
        })
        .collect()
}

/// Before a table deletion removes the text `spans`: refuse when that deletes a content control
/// locked against deletion or edits content locked against editing (#1329). With Track Changes
/// on, text not yet deleted is marked deleted instead and the structure stays (like Word deleting
/// a table with tracking on; the model has no tracked row or column deletion), returning the
/// result. With no text at all (an empty table, or one whose deletion was accepted) there is
/// nothing for Reject All to restore and the structure goes as without tracking; with only text
/// already marked deleted, nothing changes (removing it would lose what Reject All restores).
fn guard_delete(s: &mut Session, spans: &[(Pos, Pos)]) -> Result<Option<Value>, CmdError> {
    for (a, b) in spans {
        super::controls::check_delete(s, a, b)?;
    }
    if !s.doc.settings.track_changes {
        return Ok(None);
    }
    if !spans.iter().any(|(a, b)| has_text(s, a, b, true)) {
        if spans.iter().any(|(a, b)| has_text(s, a, b, false)) {
            let msg = "The table's text is already marked deleted; accept or reject the change first.";
            s.status = msg.to_string();
            return Err(CmdError::Failed(msg.to_string()));
        }
        return Ok(None);
    }
    let rid = super::new_revision(s, RevisionKind::Delete);
    for (a, b) in spans.iter().rev() {
        super::track_delete_as(s, a, b, rid)?;
    }
    if let Some((a, _)) = spans.first() {
        s.sel = Selection::caret(s.doc.clamp(a));
    }
    sel_result(s).map(Some)
}

/// Whether `a..b` holds text: any text, or (`live`) text that isn't a tracked deletion.
fn has_text(s: &Session, a: &Pos, b: &Pos, live: bool) -> bool {
    s.doc.paths_between(a, b).iter().any(|path| {
        let Some(p) = s.doc.para(a.story, path) else { return false };
        let from = if *path == a.path { a.off } else { 0 };
        let to = if *path == b.path { b.off } else { p.len() };
        p.run_ranges().any(|(r, c)| (!live || c.del.is_none()) && r.end > from && r.start < to && !r.is_empty())
    })
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

/// The selected cells as Split Cells sees them: one entry per cell, a vertically merged cell
/// once (its top row), each as (top row, grid column).
pub fn split_targets(s: &Session) -> Vec<(usize, usize)> {
    let Ok((tp, _, _)) = cell(s) else { return Vec::new() };
    let Some(t) = s.doc.table(s.sel.focus.story, &tp) else { return Vec::new() };
    let mut v: Vec<(usize, usize)> = selected_cells(s)
        .into_iter()
        .filter(|&(r, c)| t.rows.get(r).is_some_and(|row| c < row.cells.len()))
        .map(|(r, c)| (t.vertical_extent(r, c).map_or(r, |(top, _)| top), t.grid_col(r, c)))
        .collect();
    v.sort_unstable();
    v.dedup();
    v
}

/// Split Cells: the caret's cell, or the selected cells (merged first unless `mergeFirst` is
/// false), into `columns` × `rows` cells.
fn split(s: &mut Session, v: &Value) -> CmdResult {
    let cols = p::u64(v, "columns").unwrap_or(2).clamp(1, 63) as usize;
    let rows = p::u64(v, "rows").unwrap_or(1).clamp(1, 63) as usize;
    let mut cells = split_targets(s);
    let merge_first = cells.len() > 1 && p::bool(v, "mergeFirst").unwrap_or(true);
    if merge_first {
        // As in Word: the merged cell splits into at most the rows it covered.
        let mut sel_rows: Vec<usize> = selected_cells(s).iter().map(|&(r, _)| r).collect();
        sel_rows.sort_unstable();
        sel_rows.dedup();
        if rows > sel_rows.len() {
            return Err(CmdError::Params(format!("The number must be between 1 and {}.", sel_rows.len())));
        }
        merge(s, &json!({}))?;
        cells.clear();
    } else if cells.len() > 1 && rows > 1 {
        // Word greys out the row count when the cells split one by one.
        return Err(CmdError::Params("several cells split one by one keep their rows: `rows` must be 1".into()));
    }
    let (tp, r, c) = cell(s)?;
    let t = s.doc.table_mut(s.sel.focus.story, &tp)?;
    if cells.is_empty() {
        cells.push((t.vertical_extent(r, c).map_or(r, |(top, _)| top), t.grid_col(r, c)));
    }
    // As in Word: a merged cell splits into 1 to its height rows; it never adds rows.
    if let [(top, g)] = cells[..]
        && let Some((_, k)) = t.cell_at_grid(top, g).and_then(|ci| t.vertical_extent(top, ci))
        && k > 1
        && rows > k
    {
        return Err(CmdError::Params(format!("The number must be between 1 and {k}.")));
    }
    // Rightmost grid column first, bottom up: a split only adds grid columns inside the cell it
    // splits, so the cells still to split keep their grid places. Each is found again by its grid
    // column when its turn comes.
    let first = cells.first().copied();
    cells.sort_unstable_by_key(|&(top, g)| std::cmp::Reverse((g, top)));
    for &(top, g) in &cells {
        let Some(ci) = t.cell_at_grid(top, g) else { continue };
        // 1 × 1 on a cell that was already merged down changes nothing in Word, at any height; a
        // cell just merged by Merge cells before split stays one cell.
        let height = t.vertical_extent(top, ci).map_or(1, |(_, k)| k);
        if !merge_first && cols == 1 && rows == 1 && height > 1 {
            continue;
        }
        if height > 1 && rows > 1 && !height.is_multiple_of(rows) {
            return Err(CmdError::Params(format!("The number must be a divisor of {height}.")));
        }
        if !t.split_cell_grid(top, ci, cols, rows) {
            return Err(CmdError::Failed(format!("can't split into {rows} rows: the table would get too many rows")));
        }
    }
    // The caret goes to the first part of the first cell (the top of a merged cell).
    let (top, g) = first.unwrap_or((r, 0));
    let c = t.cell_at_grid(top, g).unwrap_or(c);
    fix_caret(s, &tp, top, c)
}

fn split_table(s: &mut Session, _: &Value) -> CmdResult {
    let (tp, r, _) = cell(s)?;
    if r == 0 {
        return Err(CmdError::Failed("can't split at the first row".into()));
    }
    let story = s.sel.focus.story;
    let t = s.doc.table_mut(story, &tp)?;
    let tail_rows = t.rows.split_off(r);
    // Content controls closing after the table now close after its second part.
    let close = std::mem::take(&mut t.controls.close);
    let controls = wordcraft_doc::ControlWrap { open: Vec::new(), close };
    let tail = Table { props: t.props.clone(), grid: t.grid.clone(), rows: tail_rows, controls };
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

pub(crate) fn cell_text(c: &wordcraft_doc::Cell) -> String {
    c.blocks.iter().filter_map(|b| b.as_para().map(|p| p.plain_text())).collect::<Vec<_>>().join("\n")
}

/// The language of a cell's text: that of its first paragraph with text.
fn cell_lang(doc: &wordcraft_doc::Document, c: &wordcraft_doc::Cell) -> Option<String> {
    c.blocks.iter().filter_map(|b| b.as_para()).find(|p| !p.plain_text().trim().is_empty()).and_then(|p| super::para_lang(doc, p))
}

/// Sort the table's rows by a column. Numbers are read in each cell's language (`1,5` is one and
/// a half in Polish) and sort first, numerically; other text sorts in the column's language.
fn sort_table(s: &mut Session, v: &Value) -> CmdResult {
    let col = p::u64(v, "column").unwrap_or(0) as usize;
    let desc = p::bool(v, "descending").unwrap_or(false);
    let header = p::bool(v, "header");
    let (tp, _, _) = cell(s)?;
    let t = s.doc.table(s.sel.focus.story, &tp).ok_or_else(|| CmdError::Failed("no table".into()))?;
    let skip = usize::from(header.unwrap_or(t.rows.first().is_some_and(|r| r.props.header))).min(t.rows.len());
    let column = || t.rows.iter().skip(skip).map(|r| r.cells.get(col));
    let lang = column().flatten().find_map(|c| cell_lang(&s.doc, c)).or_else(|| super::default_lang(s));
    // A total order (numbers, then text), so sorting never sees inconsistent comparisons.
    let keys: Vec<(Option<f64>, SortKey)> = column()
        .map(|c| {
            let text = c.map(cell_text).unwrap_or_default();
            let num_lang = c.and_then(|c| cell_lang(&s.doc, c)).or_else(|| lang.clone());
            let num = parse_number(text.trim(), num_lang.as_deref()).filter(|x| x.is_finite());
            (num, if num.is_some() { SortKey::default() } else { sort_key(text.trim(), lang.as_deref()) })
        })
        .collect();
    with_table(s, move |t| {
        if skip > t.rows.len() {
            return;
        }
        let body = t.rows.split_off(skip);
        let mut rows: Vec<_> = keys.iter().cloned().zip(body).collect();
        rows.sort_by(|((a, ka), _), ((b, kb), _)| {
            let o = match (a, b) {
                (Some(x), Some(y)) => x.total_cmp(y),
                (Some(_), None) => std::cmp::Ordering::Less,
                (None, Some(_)) => std::cmp::Ordering::Greater,
                (None, None) => ka.cmp(kb),
            };
            if desc { o.reverse() } else { o }
        });
        t.rows.extend(rows.into_iter().map(|(_, r)| r));
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

/// Table Layout › Data › Formula: inserts a formula field at the caret, evaluated now and again
/// by Update Field. What a formula can use is described in [`super::table_formula`]. A formula
/// that doesn't evaluate (an unknown function, a syntax error) is an error and inserts nothing.
fn formula(s: &mut Session, v: &Value) -> CmdResult {
    let (tp, r, c) = cell(s)?;
    let raw = p::str(v, "formula").unwrap_or("=SUM(ABOVE)");
    let t = s.doc.table(s.sel.focus.story, &tp).ok_or_else(|| CmdError::Failed("no table".into()))?;
    let default_lang = super::default_lang(s);
    let instr = auto_formula_instr(t, r, c, raw, default_lang.as_deref());
    let text = super::table_formula::evaluate_field_with(t, Some(&s.doc), default_lang.as_deref(), r, c, &instr)?;
    let props = s.typing_props();
    let at = s.sel.focus.clone();
    let end = s.doc.insert_object(
        &at,
        wordcraft_doc::InlineObject::Field {
            instr: instr.clone(),
            result: text.clone(),
            locked: false,
            dirty: false,
            code: false,
            form: String::new(),
        },
        &props,
    )?;
    s.sel = Selection::caret(end);
    Ok(json!({"result": text, "formula": instr}))
}

/// A formula as a field stores it: upper case, with one leading `=`. A `\#` numeric picture is
/// kept as typed.
pub(crate) fn formula_instr(raw: &str) -> String {
    let body = raw.trim();
    let body = body.strip_prefix('=').unwrap_or(body);
    match super::table_formula::split_switch(body) {
        (formula, Some(switch)) => format!("={} {switch}", formula.trim().to_ascii_uppercase()),
        (formula, None) => format!("={}", formula.trim().to_ascii_uppercase()),
    }
}

/// A bare function name (`=SUM`, no parentheses) takes its direction from the cell: ABOVE when a
/// cell above holds a number, else LEFT.
fn auto_formula_instr(t: &Table, r: usize, c: usize, raw: &str, lang: Option<&str>) -> String {
    let instr = formula_instr(raw);
    let body = instr.strip_prefix('=').unwrap_or(&instr);
    if !body.contains('(') && matches!(body, "SUM" | "AVERAGE" | "COUNT" | "MAX" | "MIN" | "PRODUCT") {
        let g = t.grid_col(r, c);
        let has_above = (0..r).rev().any(|row| {
            t.cell_at_grid(row, g)
                .and_then(|ci| t.rows.get(row)?.cells.get(ci))
                .and_then(|cl| super::table_formula::cell_number_in(cl, lang))
                .is_some()
        });
        let dir = if has_above { "ABOVE" } else { "LEFT" };
        format!("={body}({dir})")
    } else {
        instr
    }
}

/// Ctrl+A in a table, as in Word: the cell, then its row, then the table. A nested table that
/// is already fully selected grows to the cell around it. Returns `None` when the next step is
/// the whole story (not in a table, or the top-level table is already selected).
pub(crate) fn grow_select_all(s: &mut Session) -> Option<CmdResult> {
    let story = s.sel.focus.story;
    if let Some(cr) = s.cell_range().filter(|cr| cr.story == story).cloned() {
        let tp = cr.table.clone();
        let (last_row, last_col, one_cell) = {
            let t = s.doc.table(story, &tp)?;
            let last_row = t.rows.len().saturating_sub(1);
            let last_col = t.cols().saturating_sub(1);
            let one_cell = cr.rows.0 == cr.rows.1
                && t.rows.get(cr.rows.0).is_some_and(|row| (0..row.cells.len()).any(|c| grid_cols(t, cr.rows.0, c) == cr.cols));
            (last_row, last_col, one_cell)
        };
        if cr.rows == (0, last_row) && cr.cols == (0, last_col) {
            return tp.cell().map(|(otp, or, oc)| select_cells(s, &otp, Some((or, Some(oc)))));
        }
        if cr.rows.0 == cr.rows.1 && cr.cols == (0, last_col) {
            return Some(select_cells(s, &tp, None));
        }
        if one_cell {
            return Some(select_cells(s, &tp, Some((cr.rows.0, None))));
        }
        return Some(select_cells(s, &tp, None));
    }
    let (a, b) = s.sel.ordered();
    let (tp, r, c) = a.path.cell()?;
    if b.path.cell() != Some((tp.clone(), r, c)) {
        return None;
    }
    Some(select_cells(s, &tp, Some((r, Some(c)))))
}

fn select_cells(s: &mut Session, tp: &Path, which: Option<(usize, Option<usize>)>) -> CmdResult {
    let story = s.sel.focus.story;
    let Some(t) = s.doc.table(story, tp) else { return sel_result(s) };
    let (last_row, last_col) = (t.rows.len().saturating_sub(1), t.cols().saturating_sub(1));
    let (rows, cols) = match which {
        None => ((0, last_row), (0, last_col)),
        Some((r, None)) => ((r, r), (0, last_col)),
        Some((r, Some(c))) => ((r, r), grid_cols(t, r, c)),
    };
    select_range(s, story, tp, rows, cols)
}

/// The grid columns cell `c` of row `r` covers (first, last).
pub(super) fn grid_cols(t: &Table, r: usize, c: usize) -> (usize, usize) {
    let g = t.grid_col(r, c);
    let span = t.rows.get(r).and_then(|row| row.cells.get(c)).map_or(1, wordcraft_doc::Cell::span);
    (g, g + span - 1)
}

/// Table Layout › Table › Select › Select Column: the grid columns of the selected cells, every row.
fn select_column(s: &mut Session, _: &Value) -> CmdResult {
    let (tp, r, c) = cell(s)?;
    let story = s.sel.focus.story;
    let t = s.doc.table(story, &tp).ok_or_else(|| CmdError::Disabled("the cursor isn't in a table".into()))?;
    let (mut g0, mut g1) = grid_cols(t, r, c);
    for (r, c) in selected_cells(s) {
        let (a, b) = grid_cols(t, r, c);
        (g0, g1) = (g0.min(a), g1.max(b));
    }
    let rows = (0, t.rows.len().saturating_sub(1));
    select_range(s, story, &tp, rows, (g0, g1))
}

/// Select whole cells of table `tp`: rows `rows`, and of them every cell reaching into grid
/// columns `cols` (both inclusive, either order). The selection runs from the first cell's start
/// to the last cell's end; a block of the cells' paragraphs (`CellRange`) says which cells they
/// are, so the app shades them and, when they aren't one run of the text (part of a column),
/// edits and formatting leave the cells between the corners alone.
pub(crate) fn select_range(s: &mut Session, story: StoryRef, tp: &Path, rows: (usize, usize), cols: (usize, usize)) -> CmdResult {
    let t = s.doc.table(story, tp).ok_or_else(|| CmdError::Failed("there's no table there".into()))?;
    let (last_row, last_col) = (t.rows.len().saturating_sub(1), t.cols().saturating_sub(1));
    let rows = (rows.0.min(rows.1).min(last_row), rows.0.max(rows.1).min(last_row));
    let cols = (cols.0.min(cols.1).min(last_col), cols.0.max(cols.1).min(last_col));
    let stream = rows.0 == rows.1 || (cols.0 == 0 && cols.1 == last_col);
    let range = crate::CellRange { story, table: tp.clone(), rows, cols, stream };
    let picked: std::collections::BTreeSet<(usize, usize)> = range.cells(t).into_iter().collect();
    // Every paragraph of the picked cells (nested tables' too), in document order.
    let depth = tp.0.len();
    let in_picked = |q: &Path| match (q.0.get(depth), q.0.get(depth + 1)) {
        (Some(r), Some(c)) => q.0.len() > depth + 2 && q.0.starts_with(&tp.0) && picked.contains(&(*r as usize, *c as usize)),
        _ => false,
    };
    let segments: Vec<(Pos, Pos)> = s
        .doc
        .para_paths(story)
        .into_iter()
        .filter(|q| in_picked(q))
        .map(|q| {
            let len = s.doc.para(story, &q).map_or(0, Paragraph::len);
            (Pos { story, path: q.clone(), off: 0 }, Pos { story, path: q, off: len })
        })
        .collect();
    let (Some(first), Some(last)) = (segments.first(), segments.last()) else {
        return Err(CmdError::Failed("there are no cells to select there".into()));
    };
    // The end corner is in the last cell's own last paragraph (not a nested table's), so it
    // names the cell.
    let focus = segments.iter().rev().find(|(_, b)| b.path.0.len() == depth + 3).map_or(&last.1, |(_, b)| b).clone();
    let sel = Selection { anchor: first.0.clone(), focus };
    s.sel = sel.clone();
    s.column = Some(crate::ColumnBlock { sel, anchor_x: 0.0, focus_x: 0.0, segments, cells: Some(range), pieces: None });
    s.column_mode = false;
    s.widened = None;
    s.goal_x = None;
    s.pending = None;
    sel_result(s)
}

/// Extend the selection from its anchor to `to` the way Word does around tables: from one cell
/// into another of the same table it takes whole cells, the rectangle between them; between a
/// table's cell and text outside the table, whole rows, from the cell's row out to the table's
/// edge facing the other end. Where the selection really runs is kept (`Session::sel_ends`), so
/// extending it again goes on from there.
pub(crate) fn extend_selection(s: &mut Session, to: Pos) {
    let ends = Selection { anchor: s.sel.anchor.clone(), focus: to };
    // Shift+Up and Shift+Down keep their goal column.
    let goal_x = s.goal_x;
    s.sel = ends.clone();
    s.column = None;
    widen(s, &ends);
    s.widened = if s.sel == ends { None } else { Some(crate::session::Widened { sel: s.sel.clone(), ends }) };
    s.goal_x = goal_x;
}

/// Shift+Right (Shift+Left in right-to-left text) out of the cell the selection is in takes that
/// cell whole first, as Word does (its end-of-cell mark); the next press goes on into the next
/// cell. `ends` is where the selection ran before the step, `to` where the step went, `cells` the
/// whole cells selected before it. Whether it took the cell.
pub(crate) fn select_cell_first(s: &mut Session, ends: &Selection, to: &Pos, cells: Option<&crate::CellRange>) -> bool {
    let (a, f) = (&ends.anchor, &ends.focus);
    if a.story != f.story || to.story != f.story || to <= f {
        return false;
    }
    // The innermost cell both ends are in.
    let mut cell = None;
    let mut i = 0;
    while let (Some(x), Some(y)) = (cell_at(&a.path.0, i), cell_at(&f.path.0, i)) {
        if x != y {
            break;
        }
        cell = Some(x);
        i += 3;
    }
    let Some((tp, r, c)) = cell else { return false };
    // Still in the cell, or that cell alone is already selected: nothing to take first.
    if f.path.0.get(..tp.0.len() + 2).is_some_and(|prefix| to.path.0.starts_with(prefix)) {
        return false;
    }
    let Some(cols) = s.doc.table(f.story, &tp).map(|t| grid_cols(t, r, c)) else { return false };
    if cells.is_some_and(|cr| cr.story == f.story && cr.table == tp && cr.rows == (r, r) && cr.cols == cols) {
        return false;
    }
    select_range(s, f.story, &tp, (r, r), cols).is_ok()
}

/// Widen the selection between `ends` (already `s.sel`) by Word's table rule (see
/// `extend_selection`).
fn widen(s: &mut Session, ends: &Selection) {
    let (a, f) = (&ends.anchor, &ends.focus);
    if a.story != f.story {
        return;
    }
    // Down through the tables both ends are in, one nesting level (three indexes) at a time.
    let mut i = 0;
    while let (Some(ab), Some(fb)) = (a.path.0.get(i), f.path.0.get(i)) {
        match (cell_at(&a.path.0, i), cell_at(&f.path.0, i)) {
            // The same cell: look inside it.
            (Some(x), Some(y)) if ab == fb && x == y => i += 3,
            // Two cells of one table: the rectangle between them.
            (Some((tp, ra, ca)), Some((_, rf, cf))) if ab == fb => {
                let cols = s.doc.table(a.story, &tp).map(|t| {
                    let ((g0, g1), (h0, h1)) = (grid_cols(t, ra, ca), grid_cols(t, rf, cf));
                    (g0.min(h0), g1.max(h1))
                });
                if let Some(cols) = cols {
                    // A table without paragraphs keeps the plain selection.
                    let _ = select_range(s, a.story, &tp, (ra, rf), cols);
                }
                return;
            }
            // Different blocks: an end in a table takes its whole row, out to the table's edge
            // facing the other end.
            (x, y) if ab != fb => {
                let forward = ab < fb;
                if let Some((tp, r, _)) = x
                    && let Some((start, end)) = row_ends(&s.doc, a.story, &tp, r)
                {
                    s.sel.anchor = if forward { start } else { end };
                }
                if let Some((tp, r, _)) = y
                    && let Some((start, end)) = row_ends(&s.doc, f.story, &tp, r)
                {
                    s.sel.focus = if forward { end } else { start };
                }
                return;
            }
            // One paragraph.
            _ => return,
        }
    }
}

/// The cell of the table at index `i` of paragraph path `p`, if the paragraph is in one there:
/// (the table's path, row, cell).
fn cell_at(p: &[u32], i: usize) -> Option<(Path, usize, usize)> {
    match (p.get(..=i), p.get(i + 1), p.get(i + 2)) {
        (Some(t), Some(r), Some(c)) if p.len() > i + 3 => Some((Path(t.to_vec()), *r as usize, *c as usize)),
        _ => None,
    }
}

/// Where row `r` of table `tp` starts and ends: its first cell's start and its last cell's end.
pub(crate) fn row_ends(doc: &Document, story: StoryRef, tp: &Path, r: usize) -> Option<(Pos, Pos)> {
    let mut row = tp.0.clone();
    row.push(u32::try_from(r).ok()?);
    let mut paths = doc.para_paths(story).into_iter().filter(|q| q.0.starts_with(&row));
    let first = paths.next()?;
    let last = paths.next_back().unwrap_or_else(|| first.clone());
    let len = doc.para(story, &last).map_or(0, Paragraph::len);
    Some((Pos { story, path: first, off: 0 }, Pos { story, path: last, off: len }))
}

/// The paragraphs Convert Text to Table would convert: the selected ones, as text. Refused
/// rather than dropping content when a table lies between them.
pub fn text_table_lines(s: &Session) -> Result<Vec<String>, CmdError> {
    let (a, b) = s.sel.ordered();
    if a.path.parent() != b.path.parent() || a.story != b.story {
        return Err(CmdError::Failed("select paragraphs to convert".into()));
    }
    let lines: Vec<String> =
        (a.path.last()..=b.path.last()).filter_map(|i| s.doc.para(a.story, &a.path.with_last(i)).map(|p| p.plain_text())).collect();
    // Refuse rather than drop content: a table between the paragraphs would be removed with them.
    if lines.len() != b.path.last() - a.path.last() + 1 {
        return Err(CmdError::Failed("select only paragraphs to convert".into()));
    }
    Ok(lines)
}

/// The cells Convert Text to Table makes of `lines`, row by row, and the column count. `sep`
/// `None` separates at paragraphs: each paragraph is a cell, `columns` (default 1) to a row.
/// Otherwise each paragraph starts a row; `columns` defaults to the most fields in one, and a
/// paragraph with more fields than that carries on over the next rows.
pub fn text_table_cells(lines: &[String], sep: Option<char>, columns: Option<usize>) -> (Vec<Vec<&str>>, usize) {
    let Some(sep) = sep else {
        let cols = columns.unwrap_or(1).clamp(1, wordcraft_doc::table::MAX_COLS);
        return (lines.chunks(cols).map(|ch| ch.iter().map(String::as_str).collect()).collect(), cols);
    };
    let cols = columns.unwrap_or_else(|| lines.iter().map(|l| l.split(sep).count()).max().unwrap_or(1)).max(1);
    let mut rows = Vec::new();
    for l in lines {
        let fields: Vec<&str> = l.split(sep).collect();
        rows.extend(fields.chunks(cols.min(wordcraft_doc::table::MAX_COLS)).map(<[&str]>::to_vec));
    }
    (rows, cols)
}

/// The cells Convert Text to Table makes of `paras`, row by row, and the column count, as
/// [`text_table_cells`] makes them of their text. Each cell is its field of the source paragraph
/// cut out of it, trimmed of surrounding white space, so its formatting, objects (pictures,
/// fields, notes…) and tracked changes go with it.
fn text_table_paras(paras: Vec<Paragraph>, sep: Option<char>, columns: Option<usize>) -> Result<(Vec<Vec<Paragraph>>, usize), CmdError> {
    use wordcraft_doc::para::OBJ;
    use wordcraft_doc::table::MAX_COLS;
    // Each paragraph's fields as byte ranges; an object is never a separator.
    let fields: Vec<Vec<std::ops::Range<usize>>> = paras
        .iter()
        .map(|p| {
            let mut out = Vec::new();
            let mut from = 0;
            if let Some(sep) = sep.filter(|c| *c != OBJ) {
                for (i, c) in p.text.char_indices().filter(|(_, c)| *c == sep) {
                    out.push(from..i);
                    from = i + c.len_utf8();
                }
            }
            out.push(from..p.text.len());
            out
        })
        .collect();
    let cols = match sep {
        None => columns.unwrap_or(1).clamp(1, MAX_COLS),
        Some(_) => columns.unwrap_or_else(|| fields.iter().map(Vec::len).max().unwrap_or(1)).max(1),
    };
    let per_row = cols.min(MAX_COLS);
    let mut rows: Vec<Vec<Paragraph>> = Vec::new();
    let mut row: Vec<Paragraph> = Vec::new();
    for (mut work, ranges) in paras.into_iter().zip(fields) {
        // The section break stays out of the cells.
        work.section = None;
        let mut pieces = Vec::with_capacity(ranges.len());
        // Last field first, so the earlier offsets stay valid.
        for r in ranges.into_iter().rev() {
            let field = work.text.get(r.clone()).unwrap_or("");
            let start = r.start + (field.len() - field.trim_start().len());
            let end = (r.start + field.trim_end().len()).max(start);
            let mut piece = work.split_off(start)?;
            piece.split_off(end - start)?;
            pieces.push(piece);
        }
        pieces.reverse();
        for piece in pieces {
            row.push(piece);
            if row.len() == per_row {
                rows.push(std::mem::take(&mut row));
            }
        }
        // With a separator, each paragraph starts a row.
        if sep.is_some() && !row.is_empty() {
            rows.push(std::mem::take(&mut row));
        }
    }
    if !row.is_empty() {
        rows.push(row);
    }
    Ok((rows, cols))
}

/// Word's largest Initial column width, 22 in, in points.
const MAX_INITIAL_WIDTH: f64 = 22.0 * 72.0;

fn from_text(s: &mut Session, v: &Value) -> CmdResult {
    let sep = match p::str(v, "separator") {
        None | Some("tab") => Some('\t'),
        Some("comma") => Some(','),
        Some("paragraph") => None,
        Some("other") => match p::str(v, "char").and_then(|c| c.chars().next()) {
            Some(c) if c != '\n' && c != '\r' => Some(c),
            _ => return Err(CmdError::Params("`char` (the separating character) is required with `other`".into())),
        },
        Some(x) => return Err(CmdError::Params(format!("unknown separator `{x}`"))),
    };
    let fit = match p::str(v, "autofit") {
        None | Some("initial") => None,
        Some(m @ ("fixed" | "contents" | "window")) => Some(m),
        Some(x) => return Err(CmdError::Params(format!("unknown AutoFit behavior `{x}`"))),
    };
    // Initial column width: 0 (or none) is Auto, the text width shared out; Word takes 0 to 22 in.
    let col_width = match v.get("width") {
        // Only the Initial column width uses it; AutoFit sizes the columns itself.
        _ if matches!(fit, Some("contents" | "window")) => 0.0,
        None | Some(Value::Null) => 0.0,
        Some(w) => match w.as_f64() {
            Some(w) if (0.0..=MAX_INITIAL_WIDTH).contains(&w) => w as f32,
            _ => return Err(CmdError::Params("`width` must be between 0 and 1584 pt (22 in)".into())),
        },
    };
    let columns = p::u64(v, "columns").map(|n| n.clamp(1, wordcraft_doc::table::MAX_COLS as u64) as usize);
    let (a, b) = s.sel.ordered();
    // Checks the selection: only paragraphs, a table between them is refused.
    text_table_lines(s)?;
    let paras: Vec<Paragraph> = (a.path.last()..=b.path.last()).filter_map(|i| s.doc.para(a.story, &a.path.with_last(i)).cloned()).collect();
    let (cells, cols) = text_table_paras(paras, sep, columns)?;
    // A table has at most 63 columns; a longer row would lose its last fields.
    if cols > wordcraft_doc::table::MAX_COLS {
        return Err(CmdError::Failed("a table can have at most 63 columns".into()));
    }
    if cells.len() > wordcraft_doc::table::MAX_ROWS {
        return Err(CmdError::Failed(format!("a table can have at most {} rows", wordcraft_doc::table::MAX_ROWS)));
    }
    let width = super::page::sect(s).text_width();
    let mut t = Table::new(cells.len().max(1), cols, width);
    if col_width > 0.0 {
        // Every column gets that preferred width, even past the margins (layout fits the table
        // to the text width, as Word draws it); the table itself has no preferred width.
        t.grid.iter_mut().for_each(|w| *w = col_width);
        t.rows.iter_mut().flat_map(|row| row.cells.iter_mut()).for_each(|c| c.props.width = Some(col_width));
    }
    for (r, l) in cells.into_iter().enumerate() {
        for (c, para) in l.into_iter().enumerate() {
            if let Some(cl) = t.rows.get_mut(r).and_then(|row| row.cells.get_mut(c)) {
                cl.blocks = vec![para_block(para)];
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
    match fit {
        Some(mode) => autofit(s, &json!({ "mode": mode })),
        None => sel_result(s),
    }
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
