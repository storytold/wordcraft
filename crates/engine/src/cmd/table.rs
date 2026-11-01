//! Table Design and Table Layout tabs.

use super::table_formula::parse_number;
use serde_json::{Value, json};
use wordcraft_doc::collate::{SortKey, sort_key};
use wordcraft_doc::props::{Align, Border, BorderStyle, Borders, HeightRule, Rgb, TextDirection, VAlign};
use wordcraft_doc::{Block, Document, InlineObject, Paragraph, Path, Pos, RevisionKind, StoryRef, Table, para_block};

use super::sel_result;
use crate::{CmdError, CmdResult, CommandSpec, Selection, Session, p};

mod cells;
mod clear;
mod convert;
mod layout;
mod select;
mod structure;

pub(super) use cells::grid_cols;
pub use cells::selected_cells;
use cells::{cell, with_cells, with_table};
pub(crate) use cells::{cell_text, row_ends};
pub(super) use clear::clear_range_cells;
pub(crate) use clear::{backspace_cells, clear_cells};
use clear::{cell_prefix, guard_delete, text_spans};
use convert::{formula, from_text, quick_table, sort_table, to_text};
pub use convert::{text_table_cells, text_table_lines};
use layout::{autofit, borders, fix_caret, properties, set_column_widths};
pub(crate) use layout::{sync_cell_widths, text_direction, touch_cells};
pub(crate) use select::{extend_selection, grow_select_all, select_cell_first, select_range};
use select::{select_cells, select_column};
pub use structure::split_targets;
pub(super) use structure::{col, row};
use structure::{del_col, del_row, del_table, merge, split, split_table};
pub(super) use structure::{delete_table_at, selected_table};

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
        t(CommandSpec::new("table.toText", "Convert to Text", "Table Layout › Data", to_text)
            .params(r#"{"separator"?: "tab|comma|paragraph|other", "char"?: string (with other)} (the selected rows, or the whole table)"#)),
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
            r#"{"align"?: "left|center|right", "rtl"?: bool, "width"?: pt|null, "widthPct"?: percent, "indent"?: pt, "rowHeight"?: pt|null, "rowHeightRule"?: "atLeast|exact", "allowBreak"?: bool, "headerRow"?: bool, "columnWidth"?: pt, "cellWidth"?: pt|null, "valign"?: "top|center|bottom"} (row, column and cell settings apply to the selected rows, columns and cells, or the caret's; without settings it returns the caret's current ones)"#,
        )),
        CommandSpec::new("table.fromText", "Convert Text to Table", "Insert › Tables", from_text)
            .params(r#"{"separator"?: "tab|comma|paragraph|other", "char"?: string (with other), "columns"?: n, "autofit"?: "initial|fixed|contents|window", "width"?: pt (initial column width, 0 = Auto, at most 1584)}"#),
        CommandSpec::new("table.quick", "Quick Tables", "Insert › Tables", quick_table).params(r#"{"kind"?: "calendar|tabular|matrix"}"#),
    ]
}
