use craft::behavior::IconId;
use craft::keymap::CommandRef;
use craft::menu::ribbon::{RibbonGroup, RibbonRow};
use craft::ui::buttons::{Arrangement, Button, ToggleButton};
use craft::ui::contract::{Gap, Props as _, Show as _, ShowContainer as _};
use craft::ui::layout_components::Stack;
use craft::ui::list::ThumbAspect;
use craft::ui::menu::MenuItem;
use craft::ui::tile_grid::{CaptionPlace, Tile};

use super::common::{menu_button, size_field};
use super::{WordApp, json};
use crate::equation_tab::pick_tile;

/// Chart Design and SmartArt Design › Size: the selected graphic's height and width in points. A
/// size outside the range is shown as it is and writes nothing.
fn graphic_size(row: &mut RibbonRow<'_>, app: &mut WordApp) {
    row.group(RibbonGroup::new("size", tl!("Size").to_string(), IconId::new("size").into()), |ui| {
        let (w0, h0) =
            wordcraft_engine::cmd::objects::selected(&app.session).and_then(|(_, o)| o.frame().map(|(w, h, _)| (w, h))).unwrap_or((0.0, 0.0));
        Stack::new().gap(Gap::Tight).show(ui, |ui, _| {
            for (label, key, v0) in [("H:", "height", h0), ("W:", "width", w0)] {
                size_field(ui, app, label, key, v0, false, 36.0..=2000.0);
            }
        });
    });
}

/// SmartArt Design: the selected graphic's items (Add Shape, Promote, Demote, Text Pane), its
/// layout (a gallery of previews), its colours and Reset.
pub(super) fn smart_art_design(row: &mut RibbonRow<'_>, app: &mut WordApp) {
    use wordcraft_doc::smart_art::{SmartArtColors, SmartArtLayout};
    let Some(g) = wordcraft_engine::cmd::smart_art::selected_smart_art(&app.session) else { return };
    let theme = app.session.doc.settings.theme_colors.clone();
    // The Text Pane's current item, else the last.
    let current = app.smart_art_item.filter(|i| *i < g.items.len());
    let pane_open = app.session.view.smart_art_pane;
    row.group(RibbonGroup::new("createGraphic", tl!("Create Graphic").to_string(), IconId::new("addShape").into()), |ui| {
        Button::command(CommandRef::with_params("smartArt.addShape", current.map_or(json!({}), |i| json!({"after": i}))))
            .label(tl!("Add\nShape").to_string())
            .icon(IconId::new("addShape"))
            .arrangement(Arrangement::Stacked)
            .show(ui);
        Stack::new().gap(Gap::Tight).show(ui, |ui, _| {
            let idx = current.map_or(json!({}), |i| json!({"index": i}));
            Button::command(CommandRef::with_params("smartArt.promote", idx.clone()))
                .label(tl!("Promote").to_string())
                .icon(IconId::new("promote"))
                .show(ui);
            Button::command(CommandRef::with_params("smartArt.demote", idx)).label(tl!("Demote").to_string()).icon(IconId::new("demote")).show(ui);
            ToggleButton::command(CommandRef::new("smartArt.textPane"))
                .label(tl!("Text Pane").to_string())
                .icon(IconId::new("textPane"))
                .selected(pane_open)
                .show(ui);
        });
    });
    row.group(RibbonGroup::new("layouts", tl!("Layouts").to_string(), IconId::new("smartArtLayouts").into()), |ui| {
        let layouts = SmartArtLayout::ALL;
        let on = layouts.iter().position(|l| *l == g.layout);
        let colors = g.colors;
        let theme = &theme;
        let line = ui.visuals().weak_text_color();
        let picked = pick_tile(ui, tl!("Layouts"), 4, CaptionPlace::None, on, layouts.len(), |at| match layouts.get(at).copied() {
            Some(layout) => Tile::new(tl!(layout.label()).to_string())
                .thumb(ThumbAspect::new(46, 31), move |cx, r| crate::dialogs_smart_art::preview(cx, r, layout, colors, theme, line)),
            None => Tile::new(String::new()),
        });
        // A pick of the current layout does nothing.
        if let Some(layout) = picked.and_then(|i| layouts.get(i)).copied().filter(|l| *l != g.layout) {
            let _ = app.run("smartArt.layout", json!({"layout": layout.id()}));
        }
    });
    row.group(RibbonGroup::new("diagramStyles", tl!("Diagram Styles").to_string(), IconId::new("smartArtColors").into()), |ui| {
        let theme = &theme;
        let g = &g;
        let face = Button::new(tl!("Change\nColors").to_string()).icon(IconId::new("smartArtColors")).arrangement(Arrangement::Stacked);
        let _ = menu_button(ui, face, tl!("Change\nColors").to_string(), None, |ui, _| {
            let custom = g.has_custom_colors();
            for colors in SmartArtColors::ALL {
                MenuItem::command(CommandRef::with_params("smartArt.colors", json!({"variant": colors.id()})))
                    .label(crate::dialogs_smart_art::colors_label(colors))
                    .radio_mark(g.colors == colors && !custom)
                    .sample(move |cx, r| crate::dialogs_smart_art::swatches(cx, r, colors, theme))
                    .show(ui);
            }
            // Active while items carry their own colours; set in the Text Pane.
            let mut row = MenuItem::new(tl!("Custom").to_string())
                .enabled(custom)
                .radio_mark(custom)
                .sample(move |cx, r| crate::dialogs_smart_art::custom_swatches(cx, r, g, theme));
            if !custom {
                row = row.tooltip(tl!("Set the colors in the Text Pane to use custom colors.").to_string());
            }
            row.show(ui);
        });
    });
    row.group(RibbonGroup::new("reset", tl!("Reset").to_string(), IconId::new("resetGraphic").into()), |ui| {
        Button::command(CommandRef::new("smartArt.reset"))
            .label(tl!("Reset\nGraphic").to_string())
            .icon(IconId::new("resetGraphic"))
            .arrangement(Arrangement::Stacked)
            .show(ui);
    });
    graphic_size(row, app);
}

/// Chart Design: the selected chart's elements (title, legend, data labels), its data and type,
/// and its size.
pub(super) fn chart_design(row: &mut RibbonRow<'_>, app: &mut WordApp) {
    let Some(chart) = wordcraft_engine::cmd::chart::selected_chart(&app.session) else { return };
    row.group(RibbonGroup::new("chartLayouts", tl!("Chart Layouts").to_string(), IconId::new("chartTitle").into()), |ui| {
        let chart = &chart;
        let face = Button::new(tl!("Chart Title").to_string()).icon(IconId::new("chartTitle")).arrangement(Arrangement::Stacked);
        let _ = menu_button(ui, face, tl!("Chart Title").to_string(), None, |ui, _| {
            let text = chart.title.clone().unwrap_or_else(|| tl!("Chart Title").to_string());
            for (label, on, params) in
                [("None", chart.title.is_none(), json!({"text": null})), ("Above Chart", chart.title.is_some(), json!({"text": text}))]
            {
                MenuItem::command(CommandRef::with_params("chart.title", params)).label(tl!(label).to_string()).radio_mark(on).show(ui);
            }
        });
        let face = Button::new(tl!("Legend").to_string()).icon(IconId::new("legend")).arrangement(Arrangement::Stacked);
        let _ = menu_button(ui, face, tl!("Legend").to_string(), None, |ui, _| {
            MenuItem::command(CommandRef::with_params("chart.legend", json!({"pos": "none"})))
                .label(tl!("None").to_string())
                .radio_mark(chart.legend.is_none())
                .show(ui);
            for (label, pos) in [
                ("Right", wordcraft_doc::chart::LegendPos::Right),
                ("Top", wordcraft_doc::chart::LegendPos::Top),
                ("Left", wordcraft_doc::chart::LegendPos::Left),
                ("Bottom", wordcraft_doc::chart::LegendPos::Bottom),
            ] {
                MenuItem::command(CommandRef::with_params("chart.legend", json!({"pos": pos.id()})))
                    .label(tl!(label).to_string())
                    .radio_mark(chart.legend == Some(pos))
                    .show(ui);
            }
        });
        let face = Button::new(tl!("Data Labels").to_string()).icon(IconId::new("dataLabels")).arrangement(Arrangement::Stacked);
        let _ = menu_button(ui, face, tl!("Data Labels").to_string(), None, |ui, _| {
            for (label, on) in [("None", false), ("Show", true)] {
                MenuItem::command(CommandRef::with_params("chart.dataLabels", json!({"on": on})))
                    .label(tl!(label).to_string())
                    .radio_mark(chart.data_labels == on)
                    .show(ui);
            }
        });
    });
    row.group(RibbonGroup::new("data", tl!("Data").to_string(), IconId::new("chartData").into()), |ui| {
        Button::command(CommandRef::new("chart.editData"))
            .label(tl!("Edit\nData").to_string())
            .icon(IconId::new("chartData"))
            .arrangement(Arrangement::Stacked)
            .show(ui);
    });
    row.group(RibbonGroup::new("type", tl!("Type").to_string(), IconId::new("changeChartType").into()), |ui| {
        Button::command(CommandRef::new("chart.type"))
            .label(tl!("Change\nChart Type").to_string())
            .icon(IconId::new("changeChartType"))
            .arrangement(Arrangement::Stacked)
            .show(ui);
    });
    graphic_size(row, app);
}
