use super::common::{graphic_size, stack};
use super::language::mi_check;
use super::{Rect, Sense, Stroke, Tokens, Ui, WordApp, big, group, json, menu_button, pos2, small, vec2};

/// SmartArt Design: the selected graphic's items (Add Shape, Promote, Demote, Text Pane), its
/// layout (a gallery of previews), its colours and Reset.
pub(super) fn smart_art_design(app: &mut WordApp, ui: &mut Ui) {
    use wordcraft_doc::smart_art::{SmartArtColors, SmartArtLayout};
    let Some(g) = wordcraft_engine::cmd::smart_art::selected_smart_art(&app.session) else { return };
    let theme = app.session.doc.settings.theme_colors.clone();
    // The Text Pane's current item, else the last.
    let current = app.smart_art_item.filter(|i| *i < g.items.len());
    group(ui, "Create Graphic", None, app, |ui, app| {
        big(ui, app, "addShape", "Add\nShape", "smartArt.addShape", current.map_or(json!({}), |i| json!({"after": i})), false);
        stack(ui, |ui| {
            let idx = current.map_or(json!({}), |i| json!({"index": i}));
            small(ui, app, "promote", Some("Promote"), "Promote", crate::widgets::ButtonAction::new("smartArt.promote", idx.clone(), false));
            small(ui, app, "demote", Some("Demote"), "Demote", crate::widgets::ButtonAction::new("smartArt.demote", idx, false));
            small(
                ui,
                app,
                "textPane",
                Some("Text Pane"),
                "Text Pane",
                crate::widgets::ButtonAction::toggle("smartArt.textPane", json!({}), app.session.view.smart_art_pane),
            );
        });
    });
    group(ui, "Layouts", None, app, |ui, app| {
        let t = Tokens::get(ui.ctx());
        egui::Grid::new("smart_art_layout_gallery").spacing(vec2(2.0, 2.0)).show(ui, |ui| {
            for (k, layout) in SmartArtLayout::ALL.into_iter().enumerate() {
                let (r, resp) = ui.allocate_exact_size(vec2(46.0, 31.0), Sense::click());
                let on = g.layout == layout;
                let fill = if on {
                    t.checked
                } else if resp.hovered() {
                    t.hover
                } else {
                    t.ribbon
                };
                ui.painter().rect(r, 3.0, fill, Stroke::new(1.0, if on { t.accent } else { t.border }), egui::StrokeKind::Inside);
                crate::dialogs_smart_art::preview(ui.painter(), r.shrink(2.0), layout, g.colors, &theme, t.text_dim);
                resp.widget_info(|| egui::WidgetInfo::selected(egui::Role::Button, true, on, tl!(layout.label())));
                if resp.on_hover_text(tl!(layout.label())).clicked() && !on {
                    let _ = app.run("smartArt.layout", json!({"layout": layout.id()}));
                }
                if k % 4 == 3 {
                    ui.end_row();
                }
            }
        });
    });
    group(ui, "SmartArt Styles", None, app, |ui, app| {
        menu_button(
            ui,
            app,
            crate::widgets::MenuButtonOptions::new("smartArtColors", Some("Change\nColors"), "Change Colors", true, ""),
            |ui, app| {
                for colors in SmartArtColors::ALL {
                    let on = g.colors == colors;
                    let resp = ui.add(egui::Button::new(crate::dialogs_smart_art::colors_label(colors)).selected(on).min_size(vec2(200.0, 24.0)));
                    let sw = Rect::from_min_size(pos2(resp.rect.right() - 74.0, resp.rect.center().y - 6.0), vec2(66.0, 12.0));
                    crate::dialogs_smart_art::swatches(ui.painter(), sw, colors, &theme);
                    if resp.clicked() {
                        let _ = app.run("smartArt.colors", json!({"variant": colors.id()}));
                        ui.close();
                    }
                }
            },
        );
    });
    group(ui, "Reset", None, app, |ui, app| {
        big(ui, app, "resetGraphic", "Reset\nGraphic", "smartArt.reset", json!({}), false);
    });
    graphic_size(ui, app);
}

/// Chart Design: the selected chart's elements (title, legend, data labels), its data and type,
/// and its size.
pub(super) fn chart_design(app: &mut WordApp, ui: &mut Ui) {
    let Some(chart) = wordcraft_engine::cmd::chart::selected_chart(&app.session) else { return };
    group(ui, "Chart Layouts", None, app, |ui, app| {
        menu_button(ui, app, crate::widgets::MenuButtonOptions::new("chartTitle", Some("Chart Title"), "Chart Title", true, ""), |ui, app| {
            mi_check(ui, app, "None", chart.title.is_none(), "chart.title", json!({"text": null}));
            let text = chart.title.clone().unwrap_or_else(|| tl!("Chart Title").to_string());
            mi_check(ui, app, "Above Chart", chart.title.is_some(), "chart.title", json!({"text": text}));
        });
        menu_button(ui, app, crate::widgets::MenuButtonOptions::new("legend", Some("Legend"), "Legend", true, ""), |ui, app| {
            mi_check(ui, app, "None", chart.legend.is_none(), "chart.legend", json!({"pos": "none"}));
            for (label, pos) in [
                ("Right", wordcraft_doc::chart::LegendPos::Right),
                ("Top", wordcraft_doc::chart::LegendPos::Top),
                ("Left", wordcraft_doc::chart::LegendPos::Left),
                ("Bottom", wordcraft_doc::chart::LegendPos::Bottom),
            ] {
                mi_check(ui, app, label, chart.legend == Some(pos), "chart.legend", json!({"pos": pos.id()}));
            }
        });
        menu_button(ui, app, crate::widgets::MenuButtonOptions::new("dataLabels", Some("Data Labels"), "Data Labels", true, ""), |ui, app| {
            mi_check(ui, app, "None", !chart.data_labels, "chart.dataLabels", json!({"on": false}));
            mi_check(ui, app, "Show", chart.data_labels, "chart.dataLabels", json!({"on": true}));
        });
    });
    group(ui, "Data", None, app, |ui, app| {
        big(ui, app, "chartData", "Edit\nData", "chart.editData", json!({}), false);
    });
    group(ui, "Type", None, app, |ui, app| {
        big(ui, app, "changeChartType", "Change\nChart Type", "chart.type", json!({}), false);
    });
    graphic_size(ui, app);
}
