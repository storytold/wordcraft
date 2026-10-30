use super::common::{mi, stack};
use super::language::mi_check;
use super::{
    Align2, MeasureExt, Rect, Remember, Sense, Stroke, Target, Tokens, Ui, Value, WordApp, big, group, json, menu_button, pos2, regular, small,
    split, vec2,
};
use wordcraft_doc::props::BorderStyle;

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
        // Line Style, Line Weight and Pen Color set the pen; the Borders menu and the Border
        // Painter draw with it (#1740).
        let pen = app.session.table_pen;
        stack(ui, |ui| {
            menu_button(
                ui,
                app,
                crate::widgets::MenuButtonOptions::new("lineStyle", Some("Line Style"), "Line Style", false, "table.borderStyle"),
                |ui, app| {
                    for (style, label) in std::iter::once((BorderStyle::None, "No Border")).chain(crate::dialogs_table_style::LINE_STYLES) {
                        mi_sample(ui, app, tl!(label), pen.style == style, "table.borderStyle", json!({"value": style.name()}), |p, r, c| {
                            line_sample(p, r, style, 1.0, c);
                        });
                    }
                },
            );
            menu_button(
                ui,
                app,
                crate::widgets::MenuButtonOptions::new("lineWeight", Some("Line Weight"), "Line Weight", false, "table.borderWidth"),
                |ui, app| {
                    for w in wordcraft_engine::cmd::table::BORDER_WIDTHS {
                        mi_sample(ui, app, &weight_label(w), (pen.width - w).abs() < 0.01, "table.borderWidth", json!({"value": w}), |p, r, c| {
                            line_sample(p, r, BorderStyle::Single, (w * 1.4).clamp(1.0, 8.0), c);
                        });
                    }
                },
            );
            menu_button(
                ui,
                app,
                crate::widgets::MenuButtonOptions::new("penColor", Some("Pen Color"), "Pen Color", false, "table.borderColor"),
                |ui, app| {
                    mi_check(ui, app, "Automatic", pen.color.is_none(), "table.borderColor", json!({"color": null}));
                    let theme = app.session.doc.settings.theme_colors.clone();
                    crate::colors::menu(ui, app, &theme, Target::cmd("table.borderColor"));
                },
            );
        });
        // Each entry carries the pen, so a recorded macro step draws the same line when played.
        menu_button(ui, app, crate::widgets::MenuButtonOptions::new("borders", Some("Borders"), "Borders", true, "table.borders"), |ui, app| {
            for (l, k) in BORDER_SIDES {
                mi(ui, app, l, "table.borders", pen.params(k));
            }
            ui.separator();
            let drawing = app.canvas.table_tool == Some(crate::table_pen::TableTool::Draw);
            mi_check(ui, app, "Draw Table", drawing, "table.draw", json!({}));
            let gridlines = app.session.view.table_gridlines;
            mi_check(ui, app, "View Gridlines", gridlines, "table.viewGridlines", json!({}));
        });
        big(ui, app, "borderPainter", "Border\nPainter", "table.borderPainter", pen.params("all"), false);
    });
}

/// The Borders menu's sides in Word's order: label and the `table.borders` kind.
const BORDER_SIDES: [(&str, &str); 10] = [
    ("All Borders", "all"),
    ("Outside Borders", "outside"),
    ("Inside Borders", "inside"),
    ("No Border", "none"),
    ("Top Border", "top"),
    ("Bottom Border", "bottom"),
    ("Left Border", "left"),
    ("Right Border", "right"),
    ("Inside Horizontal Border", "insideH"),
    ("Inside Vertical Border", "insideV"),
];

/// A line weight as Word's menu writes it: ¼ pt, 1½ pt, 6 pt.
pub(super) fn weight_label(w: f32) -> String {
    let w = if w.is_finite() { w.clamp(0.0, 100.0) } else { 0.0 };
    let whole = w.trunc() as u32;
    let frac = match ((w - w.trunc()) * 4.0).round() as u32 {
        1 => "¼",
        2 => "½",
        3 => "¾",
        _ => "",
    };
    let n = if whole == 0 && !frac.is_empty() { frac.to_string() } else { format!("{whole}{frac}") };
    crate::i18n::fmt(tl!("{n} pt"), &[("n", &n)])
}

/// A short line in `style`, `width` pixels thick, as the Line Style and Line Weight menus show
/// it (nothing for No Border).
fn line_sample(p: &egui::Painter, r: Rect, style: BorderStyle, width: f32, c: egui::Color32) {
    let y = r.center().y;
    let (x0, x1) = (r.min.x, r.max.x);
    let seg = |dy: f32, w: f32| {
        p.line_segment([pos2(x0, y + dy), pos2(x1, y + dy)], Stroke::new(w, c));
    };
    let dashes = |dash: &[f32], gap: &[f32]| {
        p.extend(egui::Shape::dashed_line_with_offset(&[pos2(x0, y), pos2(x1, y)], Stroke::new(width, c), dash, gap, 0.0));
    };
    match style {
        BorderStyle::None => {}
        BorderStyle::Single => seg(0.0, width),
        BorderStyle::Thick => seg(0.0, width * 2.5),
        BorderStyle::Double => {
            seg(-1.5, width);
            seg(1.5, width);
        }
        BorderStyle::Triple => {
            seg(-2.5, width);
            seg(0.0, width);
            seg(2.5, width);
        }
        BorderStyle::Dotted => dashes(&[1.5], &[2.0]),
        BorderStyle::Dashed => dashes(&[6.0], &[3.0]),
        BorderStyle::DotDash => dashes(&[6.0, 1.5], &[3.0, 3.0]),
        BorderStyle::Wave => {
            let n = ((x1 - x0) / 3.0).max(2.0) as usize;
            let pts: Vec<egui::Pos2> = (0..=n).map(|i| pos2(x0 + (x1 - x0) * i as f32 / n as f32, y + if i % 2 == 0 { -1.5 } else { 1.5 })).collect();
            p.add(egui::Shape::line(pts, Stroke::new(width, c)));
        }
    }
}

/// A menu item with a drawn sample before its (already translated) text; `checked` marks the
/// current choice.
fn mi_sample(
    ui: &mut Ui,
    app: &mut WordApp,
    text: &str,
    checked: bool,
    id: &str,
    params: Value,
    sample: impl FnOnce(&egui::Painter, Rect, egui::Color32),
) {
    let t = Tokens::get(ui.ctx());
    let enabled = crate::widgets::enabled(app, id);
    let (r, resp) = ui.allocate_exact_size(vec2(220.0, 22.0), Sense::click());
    crate::widgets::bg(ui, r, &resp, checked, &t);
    let c = if enabled { t.text } else { t.text_disabled };
    if checked {
        ui.painter().text(pos2(r.min.x + 6.0, r.center().y), Align2::LEFT_CENTER, "✓", regular(12.0), c);
    }
    sample(ui.painter(), Rect::from_min_size(pos2(r.min.x + 22.0, r.min.y + 3.0), vec2(60.0, 16.0)), c);
    ui.painter().text(pos2(r.min.x + 92.0, r.center().y), Align2::LEFT_CENTER, text, egui::TextStyle::Button.resolve(ui.style()), c);
    resp.widget_info(|| egui::WidgetInfo { selected: Some(checked), ..egui::WidgetInfo::labeled(egui::Role::Button, enabled, text) });
    if enabled && resp.clicked() {
        crate::widgets::run_from_ribbon(app, id, params);
        ui.close();
    }
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
