use super::common::{mi, stack};
use super::{MeasureExt, Remember, Target, Ui, WordApp, big, group, json, menu_button, small, split, vec2};

pub(super) fn table_design(app: &mut WordApp, ui: &mut Ui) {
    let look = app.session.sel.focus.path.cell().and_then(|(tp, _, _)| app.session.doc.table(app.session.sel.focus.story, &tp).map(|t| t.props.look));
    group(ui, "Table Style Options", None, app, |ui, app| {
        let Some(l) = look else { return };
        egui::Grid::new("look").show(ui, |ui| {
            for (row, items) in [
                [("Header Row", "headerRow", l.header_row), ("First Column", "firstColumn", l.first_column)],
                [("Total Row", "totalRow", l.total_row), ("Last Column", "lastColumn", l.last_column)],
                [("Banded Rows", "bandedRows", l.banded_rows), ("Banded Columns", "bandedColumns", l.banded_columns)],
            ]
            .iter()
            .enumerate()
            {
                let _ = row;
                for (label, key, val) in items {
                    let mut v = *val;
                    if ui.checkbox(&mut v, tl!(label)).changed() {
                        let _ = app.run("table.look", json!({ *key: v }));
                    }
                }
                ui.end_row();
            }
        });
    });
    group(ui, "Table Styles", None, app, |ui, app| {
        // The document's own styles first, so a new one shows without scrolling.
        let mut styles: Vec<(bool, String, String)> = app
            .session
            .doc
            .styles
            .styles
            .iter()
            .filter(|s| s.kind == wordcraft_doc::StyleKind::Table && !s.hidden)
            .map(|s| (s.builtin, s.id.clone(), crate::i18n::style_name(s)))
            .collect();
        styles.sort_by_key(|(builtin, _, _)| *builtin);
        egui::ScrollArea::horizontal().max_width(420.0).show(ui, |ui| {
            ui.horizontal(|ui| {
                for (_, id, name) in styles {
                    if crate::previews::table_style_tile(ui, app, &id).on_hover_text(name).clicked() {
                        let _ = app.run("table.style", json!({"style": id}));
                    }
                }
            });
        });
        stack(ui, |ui| {
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
            menu_button(
                ui,
                app,
                crate::widgets::MenuButtonOptions::new("styles", Some("Styles"), "Table Styles", false, "table.style"),
                |ui, app| {
                    mi(ui, app, "New Table Style…", "ui.dialog", json!({"name": "newTableStyle"}));
                    if ui.add_enabled(can_modify, egui::Button::new(tl!("Modify Table Style…")).min_size(vec2(200.0, 0.0))).clicked() {
                        let _ = app.run("ui.dialog", json!({"name": "modifyTableStyle"}));
                        ui.close();
                    }
                    if ui.add_enabled(can_delete, egui::Button::new(tl!("Delete Table Style")).min_size(vec2(200.0, 0.0))).clicked() {
                        if let Err(e) = app.run("table.deleteStyle", json!({})) {
                            app.status(e);
                        }
                        ui.close();
                    }
                },
            );
            let sw = wordcraft_doc::Rgb::parse(&app.canvas.last_shading).map(crate::theme::c32);
            split(
                ui,
                app,
                "shadingNoBar",
                "Shading",
                crate::widgets::ButtonAction::new("table.shading", json!({"color": app.canvas.last_shading.clone()}), false),
                sw,
                |ui, app| {
                    mi(ui, app, "No Color", "table.shading", json!({"color": null}));
                    let theme = app.session.doc.settings.theme_colors.clone();
                    crate::colors::menu(ui, app, &theme, Target::cmd("table.shading").remember(Remember::Shading));
                },
            );
        });
    });
    group(ui, "Borders", None, app, |ui, app| {
        menu_button(ui, app, crate::widgets::MenuButtonOptions::new("borders", Some("Borders"), "Borders", true, "table.borders"), |ui, app| {
            for (l, k) in [
                ("All Borders", "all"),
                ("Outside Borders", "outside"),
                ("Inside Borders", "inside"),
                ("No Border", "none"),
                ("Top Border", "top"),
                ("Bottom Border", "bottom"),
            ] {
                mi(ui, app, l, "table.borders", json!({"kind": k}));
            }
        });
        big(ui, app, "borderPainter", "Border\nPainter", "table.borderPainter", json!({}), false);
    });
}

/// Table Layout › Cell Size: the caret cell's row height and column width, in inches. They show
/// the size on the page (a row without a set height shows the height its text gives it) and
/// editing runs `table.rowHeight` / `table.columnWidth`; dragging one is a single Undo.
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
    let unit = crate::options::unit();
    let k = unit.pt_per_unit();
    let h0 = row.height.or(row_h).unwrap_or(0.0);
    let w0 = laid.map(|x| x.w).or(stored_w).unwrap_or(0.0);
    let exact = row.height_rule == wordcraft_doc::props::HeightRule::Exact;
    // The edited value in points, and whether it continues a drag (joins the previous Undo step).
    let boxed = |ui: &mut Ui, label: &str, pt: f32| -> Option<(f32, bool)> {
        let mut out = None;
        crate::widgets::row(ui, |ui| {
            ui.add_sized(vec2(48.0, 18.0), egui::Label::new(egui::RichText::new(tl!(label)).small()));
            let mut v = pt / k;
            let r = ui.add_sized(vec2(72.0, 18.0), egui::DragValue::new(&mut v).speed(0.01).range(0.02..=22.0).measure(unit).max_decimals(2));
            if r.changed() {
                out = Some((v * k, r.dragged() && !r.drag_started()));
            }
        });
        ui.add_space(2.0);
        out
    };
    stack(ui, |ui| {
        if let Some((h, join)) = boxed(ui, "Height:", h0) {
            if join {
                app.session.join_next_undo();
            }
            let _ = app.run("table.rowHeight", json!({"height": h, "rule": if exact { "exact" } else { "atLeast" }}));
        }
        if let Some((w, join)) = boxed(ui, "Width:", w0) {
            if join {
                app.session.join_next_undo();
            }
            let _ = app.run("table.columnWidth", json!({"width": w}));
        }
    });
}

pub(super) fn table_layout(app: &mut WordApp, ui: &mut Ui) {
    group(ui, "Table", None, app, |ui, app| {
        stack(ui, |ui| {
            menu_button(
                ui,
                app,
                crate::widgets::MenuButtonOptions::new("selectTable", Some("Select"), "Select", false, "table.selectTable"),
                |ui, app| {
                    mi(ui, app, "Select Cell", "table.selectCell", json!({}));
                    mi(ui, app, "Select Column", "table.selectColumn", json!({}));
                    mi(ui, app, "Select Row", "table.selectRow", json!({}));
                    mi(ui, app, "Select Table", "table.selectTable", json!({}));
                },
            );
            let g = app.session.view.table_gridlines;
            small(
                ui,
                app,
                "gridlines",
                Some("View Gridlines"),
                "View Gridlines",
                crate::widgets::ButtonAction::new("table.viewGridlines", json!({}), g),
            );
            small(
                ui,
                app,
                "properties",
                Some("Properties"),
                "Table Properties",
                crate::widgets::ButtonAction::new("table.properties", json!({}), false),
            );
        });
    });
    group(ui, "Draw", None, app, |ui, app| {
        use crate::table_pen::TableTool;
        let tool = app.canvas.table_tool;
        crate::widgets::big_toggle(ui, app, "drawTable", "Draw\nTable", "table.draw", tool == Some(TableTool::Draw));
        crate::widgets::big_toggle(ui, app, "eraser", "Eraser", "table.eraser", tool == Some(TableTool::Erase));
    });
    group(ui, "Rows & Columns", None, app, |ui, app| {
        menu_button(
            ui,
            app,
            crate::widgets::MenuButtonOptions::new("deleteTable", Some("Delete"), "Delete", true, "table.deleteTable"),
            |ui, app| {
                mi(ui, app, "Delete Cells", "table.deleteCells", json!({}));
                mi(ui, app, "Delete Columns", "table.deleteColumn", json!({}));
                mi(ui, app, "Delete Rows", "table.deleteRow", json!({}));
                mi(ui, app, "Delete Table", "table.deleteTable", json!({}));
            },
        );
        big(ui, app, "insertAbove", "Insert\nAbove", "table.insertRowAbove", json!({}), false);
        stack(ui, |ui| {
            small(
                ui,
                app,
                "insertBelow",
                Some("Insert Below"),
                "Insert Below",
                crate::widgets::ButtonAction::new("table.insertRowBelow", json!({}), false),
            );
            small(
                ui,
                app,
                "insertLeft",
                Some("Insert Left"),
                "Insert Left",
                crate::widgets::ButtonAction::new("table.insertColumnLeft", json!({}), false),
            );
            small(
                ui,
                app,
                "insertRight",
                Some("Insert Right"),
                "Insert Right",
                crate::widgets::ButtonAction::new("table.insertColumnRight", json!({}), false),
            );
        });
    });
    group(ui, "Merge", None, app, |ui, app| {
        stack(ui, |ui| {
            small(ui, app, "merge", Some("Merge Cells"), "Merge Cells", crate::widgets::ButtonAction::new("table.merge", json!({}), false));
            small(ui, app, "splitCells", Some("Split Cells"), "Split Cells", crate::widgets::ButtonAction::new("table.split", json!({}), false));
            small(ui, app, "splitTable", Some("Split Table"), "Split Table", crate::widgets::ButtonAction::new("table.splitTable", json!({}), false));
        });
    });
    group(ui, "Cell Size", None, app, |ui, app| {
        menu_button(ui, app, crate::widgets::MenuButtonOptions::new("autofit", Some("AutoFit"), "AutoFit", true, "table.autofit"), |ui, app| {
            mi(ui, app, "AutoFit Contents", "table.autofit", json!({"mode": "contents"}));
            mi(ui, app, "AutoFit Window", "table.autofit", json!({"mode": "window"}));
            mi(ui, app, "Fixed Column Width", "table.autofit", json!({"mode": "fixed"}));
        });
        cell_size_boxes(ui, app);
        stack(ui, |ui| {
            small(
                ui,
                app,
                "distributeRows",
                Some("Distribute Rows"),
                "Distribute Rows",
                crate::widgets::ButtonAction::new("table.distributeRows", json!({}), false),
            );
            small(
                ui,
                app,
                "distributeCols",
                Some("Distribute Columns"),
                "Distribute Columns",
                crate::widgets::ButtonAction::new("table.distributeColumns", json!({}), false),
            );
        });
    });
    group(ui, "Alignment", None, app, |ui, app| {
        egui::Grid::new("cellalign").spacing(vec2(1.0, 1.0)).show(ui, |ui| {
            for row in [
                [("topLeft", "Align Top Left"), ("topCenter", "Align Top Center"), ("topRight", "Align Top Right")],
                [("centerLeft", "Align Center Left"), ("center", "Align Center"), ("centerRight", "Align Center Right")],
                [("bottomLeft", "Align Bottom Left"), ("bottomCenter", "Align Bottom Center"), ("bottomRight", "Align Bottom Right")],
            ] {
                for (v, tip) in row {
                    small(ui, app, "cellAlign", None, tip, crate::widgets::ButtonAction::new("table.cellAlign", json!({"value": v}), false));
                }
                ui.end_row();
            }
        });
        big(ui, app, "textDirection", "Text\nDirection", "table.textDirection", json!({}), false);
        big(ui, app, "cellMargins", "Cell\nMargins", "table.cellMargins", json!({}), false);
    });
    group(ui, "Data", None, app, |ui, app| {
        big(ui, app, "sort", "Sort", "table.sort", json!({}), false);
        stack(ui, |ui| {
            small(
                ui,
                app,
                "repeatHeader",
                Some("Repeat Header Rows"),
                "Repeat Header Rows",
                crate::widgets::ButtonAction::new("table.repeatHeader", json!({}), false),
            );
            small(ui, app, "toText", Some("Convert to Text"), "Convert to Text", crate::widgets::ButtonAction::new("table.toText", json!({}), false));
            small(ui, app, "formula", Some("Formula"), "Formula", crate::widgets::ButtonAction::new("table.formula", json!({}), false));
        });
    });
}
