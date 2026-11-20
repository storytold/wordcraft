use craft::behavior::IconId;
use craft::keymap::CommandRef;
use craft::menu::ribbon::{RibbonGroup, RibbonRow};
use craft::ui::buttons::{Arrangement, Button, IconButton, ToggleButton};
use craft::ui::choice::Checkbox;
use craft::ui::contract::{Bind, Gap, Props as _, Show as _, ShowContainer as _};
use craft::ui::layout_components::{Row, Stack};
use craft::ui::menu::MenuItem;
use craft::ui::text::NumberField;

use super::common::menu_button;
use super::{Ui, WordApp, json};
use crate::colors::{Remember, Target};
use crate::table_pen::TableTool;
use crate::undo_marks::apply_edits;

/// The row height and column width a Cell Size field accepts, in points (0.02 to 22 inches).
const CELL_SIZE_RANGE: std::ops::RangeInclusive<f64> = 1.44..=1584.0;

pub(super) fn table_design(row: &mut RibbonRow<'_>, app: &mut WordApp) {
    let look = app.session.sel.focus.path.cell().and_then(|(tp, _, _)| app.session.doc.table(app.session.sel.focus.story, &tp).map(|t| t.props.look));
    if let Some(l) = look {
        // Three rows of two, so Tab runs Header Row, First Column, Total Row, Last Column, Banded Rows, Banded Columns.
        let options = [
            [("Header Row", "headerRow", l.header_row), ("First Column", "firstColumn", l.first_column)],
            [("Total Row", "totalRow", l.total_row), ("Last Column", "lastColumn", l.last_column)],
            [("Banded Rows", "bandedRows", l.banded_rows), ("Banded Columns", "bandedColumns", l.banded_columns)],
        ];
        row.group(RibbonGroup::new("tableStyleOptions", tl!("Table Style Options").to_string(), IconId::new("tableStyleOptions").into()), |ui| {
            Stack::new().gap(Gap::Tight).show(ui, |ui, _| {
                for line in options {
                    Row::new().show(ui, |ui, _| {
                        for (label, key, value) in line {
                            let out = Checkbox::new(Bind::given(value)).label(tl!(label).to_string()).show(ui);
                            apply_edits(app, out.events, |app, v| {
                                let _ = app.run("table.look", json!({ key: v }));
                            });
                        }
                    });
                }
            });
        });
    }
    let current = look.and_then(|_| {
        let (tp, _, _) = app.session.sel.focus.path.cell()?;
        let id = app.session.doc.table(app.session.sel.focus.story, &tp)?.props.style.clone()?;
        app.session
            .doc
            .styles
            .get(&id)
            .filter(|st| st.kind == wordcraft_doc::StyleKind::Table)
            .map(wordcraft_engine::cmd::table_style::is_builtin_table_style)
    });
    let can_modify = current.is_some();
    // Built-in table styles can't be deleted.
    let can_delete = current == Some(false);
    row.group(RibbonGroup::new("tableStyles", tl!("Table Styles").to_string(), IconId::new("tableStyles").into()), |ui| {
        super::galleries::table_styles(ui, app);
        Stack::new().gap(Gap::Tight).show(ui, |ui, _| {
            let _ = menu_button(ui, Button::new(tl!("Styles").to_string()).icon(IconId::new("styles")), tl!("Styles").to_string(), None, |ui, _| {
                MenuItem::command(CommandRef::with_params("ui.dialog", json!({"name": "newTableStyle"})))
                    .label(tl!("New Table Style…").to_string())
                    .show(ui);
                MenuItem::command(CommandRef::with_params("ui.dialog", json!({"name": "modifyTableStyle"})))
                    .label(tl!("Modify Table Style…").to_string())
                    .enabled(can_modify)
                    .show(ui);
                MenuItem::command(CommandRef::with_params("table.deleteStyle", json!({})))
                    .label(tl!("Delete Table Style").to_string())
                    .enabled(can_delete)
                    .show(ui);
            });
            let _ = crate::colors::shading_split(ui, app, tl!("Shading").to_string(), Target::cmd("table.shading").remember(Remember::Shading));
        });
    });
    row.group(RibbonGroup::new("borders", tl!("Borders").to_string(), IconId::new("borders").into()), |ui| {
        let face = Button::new(tl!("Borders").to_string()).icon(IconId::new("borders")).arrangement(Arrangement::Stacked);
        let _ = menu_button(ui, face, tl!("Borders").to_string(), None, |ui, _| {
            for (label, kind) in [
                ("All Borders", "all"),
                ("Outside Borders", "outside"),
                ("Inside Borders", "inside"),
                ("No Border", "none"),
                ("Top Border", "top"),
                ("Bottom Border", "bottom"),
            ] {
                MenuItem::command(CommandRef::with_params("table.borders", json!({"kind": kind}))).label(tl!(label).to_string()).show(ui);
            }
        });
        Button::command(CommandRef::new("table.borderPainter"))
            .label(tl!("Border\nPainter").to_string())
            .icon(IconId::new("borderPainter"))
            .arrangement(Arrangement::Stacked)
            .show(ui);
    });
}

/// Table Layout › Cell Size: the caret cell's row height and column width. They show the size on the
/// page (a row without a set height shows the height its text gives it) and editing runs
/// `table.rowHeight` / `table.columnWidth`; a drag is one undo step.
fn cell_size_boxes(ui: &mut Ui, app: &mut WordApp) {
    let story = app.session.sel.focus.story;
    let Some((tp, r, c)) = app.session.sel.focus.path.cell() else { return };
    let Some(t) = app.session.doc.table(story, &tp) else { return };
    let row = t.rows.get(r).map(|x| x.props.clone()).unwrap_or_default();
    let stored_w = t.grid.get(t.grid_col(r, c)).copied();
    let laid = app.session.layout().pages.iter().flat_map(|pg| pg.items.iter()).find_map(|it| match it {
        wordcraft_layout::Placed::Cell { rect, table, row, cell, story: st, .. } if *table == tp && *row == r && *cell == c && *st == story => {
            Some(*rect)
        }
        _ => None,
    });
    // The row's own height: a vertically merged cell's box spans several rows, so take the
    // shortest box in this row.
    let row_h = app
        .session
        .layout()
        .pages
        .iter()
        .flat_map(|pg| pg.items.iter())
        .filter_map(|it| match it {
            wordcraft_layout::Placed::Cell { rect, table, row, story: st, .. } if *table == tp && *row == r && *st == story => Some(rect.h),
            _ => None,
        })
        .reduce(f32::min);
    let h0 = row.height.or(row_h).unwrap_or(0.0);
    let w0 = laid.map(|x| x.w).or(stored_w).unwrap_or(0.0);
    let exact = row.height_rule == wordcraft_doc::props::HeightRule::Exact;
    let (height_units, width_units) = (crate::options::lengths(app.session.units()), crate::options::lengths(app.session.units()));
    Stack::new().gap(Gap::Tight).show(ui, |ui, _| {
        let out = NumberField::length(tl!("Height:").to_string(), Bind::given(Some(f64::from(h0))), height_units).range(CELL_SIZE_RANGE).show(ui);
        apply_edits(app, out.edit.events, |app, v| {
            if let Some(h) = v {
                let _ = app.run("table.rowHeight", json!({"height": h, "rule": if exact { "exact" } else { "atLeast" }}));
            }
        });
        let out = NumberField::length(tl!("Width:").to_string(), Bind::given(Some(f64::from(w0))), width_units).range(CELL_SIZE_RANGE).show(ui);
        apply_edits(app, out.edit.events, |app, v| {
            if let Some(w) = v {
                let _ = app.run("table.columnWidth", json!({"width": w}));
            }
        });
    });
}

pub(super) fn table_layout(row: &mut RibbonRow<'_>, app: &mut WordApp) {
    row.group(RibbonGroup::new("table", tl!("Table").to_string(), IconId::new("table").into()), |ui| {
        Stack::new().gap(Gap::Tight).show(ui, |ui, _| {
            let _ =
                menu_button(ui, Button::new(tl!("Select").to_string()).icon(IconId::new("selectTable")), tl!("Select").to_string(), None, |ui, _| {
                    for (label, id) in [
                        ("Select Cell", "table.selectCell"),
                        ("Select Column", "table.selectColumn"),
                        ("Select Row", "table.selectRow"),
                        ("Select Table", "table.selectTable"),
                    ] {
                        MenuItem::command(CommandRef::new(id)).label(tl!(label).to_string()).show(ui);
                    }
                });
            ToggleButton::command(CommandRef::new("table.viewGridlines"))
                .label(tl!("View Gridlines").to_string())
                .icon(IconId::new("gridlines"))
                .selected(app.session.view.table_gridlines)
                .show(ui);
            Button::command(CommandRef::new("table.properties")).label(tl!("Properties").to_string()).icon(IconId::new("properties")).show(ui);
        });
    });
    row.group(RibbonGroup::new("draw", tl!("Draw").to_string(), IconId::new("drawTable").into()), |ui| {
        ToggleButton::command(CommandRef::new("table.draw"))
            .label(tl!("Draw\nTable").to_string())
            .icon(IconId::new("drawTable"))
            .arrangement(Arrangement::Stacked)
            .selected(app.canvas.table_tool == Some(TableTool::Draw))
            .show(ui);
        ToggleButton::command(CommandRef::new("table.eraser"))
            .label(tl!("Eraser").to_string())
            .icon(IconId::new("wordcraft.eraser"))
            .arrangement(Arrangement::Stacked)
            .selected(app.canvas.table_tool == Some(TableTool::Erase))
            .show(ui);
    });
    row.group(RibbonGroup::new("rowsColumns", tl!("Rows & Columns").to_string(), IconId::new("insertAbove").into()), |ui| {
        let face = Button::new(tl!("Delete").to_string()).icon(IconId::new("deleteTable")).arrangement(Arrangement::Stacked);
        let _ = menu_button(ui, face, tl!("Delete").to_string(), None, |ui, _| {
            for (label, id) in [
                ("Delete Cells", "table.deleteCells"),
                ("Delete Columns", "table.deleteColumn"),
                ("Delete Rows", "table.deleteRow"),
                ("Delete Table", "table.deleteTable"),
            ] {
                MenuItem::command(CommandRef::new(id)).label(tl!(label).to_string()).show(ui);
            }
        });
        Button::command(CommandRef::new("table.insertRowAbove"))
            .label(tl!("Insert\nAbove").to_string())
            .icon(IconId::new("insertAbove"))
            .arrangement(Arrangement::Stacked)
            .show(ui);
        Stack::new().gap(Gap::Tight).show(ui, |ui, _| {
            Button::command(CommandRef::new("table.insertRowBelow")).label(tl!("Insert Below").to_string()).icon(IconId::new("insertBelow")).show(ui);
            Button::command(CommandRef::new("table.insertColumnLeft")).label(tl!("Insert Left").to_string()).icon(IconId::new("insertLeft")).show(ui);
            Button::command(CommandRef::new("table.insertColumnRight"))
                .label(tl!("Insert Right").to_string())
                .icon(IconId::new("insertRight"))
                .show(ui);
        });
    });
    row.group(RibbonGroup::new("merge", tl!("Merge").to_string(), IconId::new("merge").into()), |ui| {
        Stack::new().gap(Gap::Tight).show(ui, |ui, _| {
            Button::command(CommandRef::new("table.merge")).label(tl!("Merge Cells").to_string()).icon(IconId::new("merge")).show(ui);
            Button::command(CommandRef::new("table.split")).label(tl!("Split Cells").to_string()).icon(IconId::new("splitCells")).show(ui);
            Button::command(CommandRef::new("table.splitTable")).label(tl!("Split Table").to_string()).icon(IconId::new("splitTable")).show(ui);
        });
    });
    row.group(RibbonGroup::new("cellSize", tl!("Cell Size").to_string(), IconId::new("autofit").into()), |ui| {
        let face = Button::new(tl!("AutoFit").to_string()).icon(IconId::new("autofit")).arrangement(Arrangement::Stacked);
        let _ = menu_button(ui, face, tl!("AutoFit").to_string(), None, |ui, _| {
            for (label, mode) in [("AutoFit Contents", "contents"), ("AutoFit Window", "window"), ("Fixed Column Width", "fixed")] {
                MenuItem::command(CommandRef::with_params("table.autofit", json!({"mode": mode}))).label(tl!(label).to_string()).show(ui);
            }
        });
        cell_size_boxes(ui, app);
        Stack::new().gap(Gap::Tight).show(ui, |ui, _| {
            Button::command(CommandRef::new("table.distributeRows"))
                .label(tl!("Distribute Rows").to_string())
                .icon(IconId::new("distributeRows"))
                .show(ui);
            Button::command(CommandRef::new("table.distributeColumns"))
                .label(tl!("Distribute Columns").to_string())
                .icon(IconId::new("distributeCols"))
                .show(ui);
        });
    });
    row.group(RibbonGroup::new("alignment", tl!("Alignment").to_string(), IconId::new("cellAlign").into()), |ui| {
        Stack::new().gap(Gap::Tight).show(ui, |ui, _| {
            for line in [
                [("topLeft", "Align Top Left"), ("topCenter", "Align Top Center"), ("topRight", "Align Top Right")],
                [("centerLeft", "Align Center Left"), ("center", "Align Center"), ("centerRight", "Align Center Right")],
                [("bottomLeft", "Align Bottom Left"), ("bottomCenter", "Align Bottom Center"), ("bottomRight", "Align Bottom Right")],
            ] {
                Row::new().show(ui, |ui, _| {
                    for (v, tip) in line {
                        IconButton::command(IconId::new("cellAlign"), CommandRef::with_params("table.cellAlign", json!({"value": v})))
                            .label(tl!(tip).to_string())
                            .tooltip(tl!(tip).to_string())
                            .show(ui);
                    }
                });
            }
        });
        Button::command(CommandRef::new("table.textDirection"))
            .label(tl!("Text\nDirection").to_string())
            .icon(IconId::new("textDirection"))
            .arrangement(Arrangement::Stacked)
            .show(ui);
        Button::command(CommandRef::new("table.cellMargins"))
            .label(tl!("Cell\nMargins").to_string())
            .icon(IconId::new("cellMargins"))
            .arrangement(Arrangement::Stacked)
            .show(ui);
    });
    row.group(RibbonGroup::new("data", tl!("Data").to_string(), IconId::new("sort").into()), |ui| {
        Button::command(CommandRef::new("table.sort"))
            .label(tl!("Sort").to_string())
            .icon(IconId::new("sort"))
            .arrangement(Arrangement::Stacked)
            .show(ui);
        Stack::new().gap(Gap::Tight).show(ui, |ui, _| {
            Button::command(CommandRef::new("table.repeatHeader"))
                .label(tl!("Repeat Header Rows").to_string())
                .icon(IconId::new("repeatHeader"))
                .show(ui);
            Button::command(CommandRef::new("table.toText")).label(tl!("Convert to Text").to_string()).icon(IconId::new("toText")).show(ui);
            Button::command(CommandRef::new("table.formula")).label(tl!("Formula").to_string()).icon(IconId::new("formula")).show(ui);
        });
    });
}
