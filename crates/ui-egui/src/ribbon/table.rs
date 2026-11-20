use craft::behavior::IconId;
use craft::keymap::CommandRef;
use craft::menu::ribbon::{RibbonGroup, RibbonRow};
use craft::ui::buttons::{Arrangement, Button, IconButton, ToggleButton};
use craft::ui::choice::Checkbox;
use craft::ui::contract::{Bind, Gap, Props as _, Show as _, ShowContainer as _, clicked};
use craft::ui::display::Divider;
use craft::ui::layout_components::{Row, Stack};
use craft::ui::menu::MenuItem;
use craft::ui::text::NumberField;
use egui::{Rect, Stroke, pos2};
use wordcraft_doc::props::BorderStyle;

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
        // Include the active pen in each border command so recorded macro steps keep it.
        let pen = app.session.table_pen;
        Stack::new().gap(Gap::Tight).show(ui, |ui, _| {
            menu_button(ui, Button::new(tl!("Line Style")).icon(IconId::new("borders")), tl!("Line Style"), Some("table.borderStyle"), |ui, _| {
                for (style, label) in std::iter::once((BorderStyle::None, "No Border")).chain(crate::dialogs::dialogs_table_style::LINE_STYLES) {
                    MenuItem::command(CommandRef::with_params("table.borderStyle", json!({"value": style.name()})))
                        .label(tl!(label))
                        .check(pen.style == style)
                        .sample(move |cx, rect| line_sample(&cx.painter, rect, style, 1.0, cx.ink()))
                        .show(ui);
                }
            });
            menu_button(ui, Button::new(tl!("Line Weight")).icon(IconId::new("borders")), tl!("Line Weight"), Some("table.borderWidth"), |ui, _| {
                let pixel = 1.0 / ui.ctx().pixels_per_point();
                for w in wordcraft_engine::cmd::table::BORDER_WIDTHS {
                    MenuItem::command(CommandRef::with_params("table.borderWidth", json!({"value": w})))
                        .label(weight_label(w))
                        .check((pen.width - w).abs() < 0.01)
                        .sample(move |cx, rect| line_sample(&cx.painter, rect, BorderStyle::Single, w.max(pixel), cx.ink()))
                        .show(ui);
                }
            });
            menu_button(ui, Button::new(tl!("Pen Color")).icon(IconId::new("pen")), tl!("Pen Color"), Some("table.borderColor"), |ui, cx| {
                MenuItem::command(CommandRef::with_params("table.borderColor", json!({"color": null})))
                    .label(tl!("Automatic"))
                    .check(pen.color.is_none())
                    .show(ui);
                let pick = crate::colors::menu(ui, app, Target::cmd("table.borderColor"));
                if pick.picked.is_some() || clicked(&pick.response) {
                    cx.request_close();
                }
            });
        });
        let face = Button::new(tl!("Borders").to_string()).icon(IconId::new("borders")).arrangement(Arrangement::Stacked);
        menu_button(ui, face, tl!("Borders"), None, |ui, _| {
            for (label, kind) in BORDER_SIDES {
                MenuItem::command(CommandRef::with_params("table.borders", pen.params(kind))).label(tl!(label)).show(ui);
            }
            Divider::horizontal().show(ui);
            MenuItem::command(CommandRef::new("table.draw")).label(tl!("Draw Table")).check(app.canvas.table_tool == Some(TableTool::Draw)).show(ui);
            MenuItem::command(CommandRef::new("table.viewGridlines")).label(tl!("View Gridlines")).check(app.session.view.table_gridlines).show(ui);
        });
        Button::command(CommandRef::with_params("table.borderPainter", pen.params("all")))
            .label(tl!("Border\nPainter").to_string())
            .icon(IconId::new("borderPainter"))
            .arrangement(Arrangement::Stacked)
            .show(ui);
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
        // The compound lines (thin and thick lines side by side, the gap as named).
        BorderStyle::ThinThickSmallGap | BorderStyle::ThinThickMediumGap | BorderStyle::ThinThickLargeGap => {
            seg(-1.5, width);
            seg(1.5, width * 2.0);
        }
        BorderStyle::ThickThinSmallGap | BorderStyle::ThickThinMediumGap | BorderStyle::ThickThinLargeGap => {
            seg(-1.5, width * 2.0);
            seg(1.5, width);
        }
        BorderStyle::ThinThickThinSmallGap | BorderStyle::ThinThickThinMediumGap | BorderStyle::ThinThickThinLargeGap => {
            seg(-3.0, width);
            seg(0.0, width * 2.0);
            seg(3.0, width);
        }
        BorderStyle::Wave => {
            let n = ((x1 - x0) / 3.0).max(2.0) as usize;
            let pts: Vec<egui::Pos2> = (0..=n).map(|i| pos2(x0 + (x1 - x0) * i as f32 / n as f32, y + if i % 2 == 0 { -1.5 } else { 1.5 })).collect();
            p.add(egui::Shape::line(pts, Stroke::new(width, c)));
        }
    }
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
