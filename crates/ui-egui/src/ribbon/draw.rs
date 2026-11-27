use super::common::menu_button;
use super::{WordApp, json};
use crate::colors::Target;
use craft::behavior::IconId;
use craft::keymap::CommandRef;
use craft::menu::{RibbonGroup, RibbonRow};
use craft::ui::buttons::{Arrangement, Button, SplitButton, SplitMenu, ToggleButton};
use craft::ui::contract::{Gap, Props as _, Show as _, ShowContainer as _};
use craft::ui::layout_components::Stack;
use craft::ui::menu::{Menu, MenuItem};
use craft::ui::primitives::IconMark;
use craft::ui::style::DataColor;
use wordcraft_doc::freeform::InkTool;

pub(super) fn draw(row: &mut RibbonRow<'_>, app: &mut WordApp) {
    use wordcraft_engine::cmd::draw::{DrawMode, lasso_selection};
    let mode = app.session.view.draw.mode;
    // The pen the colour and thickness menus change: the one in use, else the pen.
    let tool = mode.pen().unwrap_or(InkTool::Pen);
    let set = app.session.view.draw.settings(tool);
    // The gallery pen in use, if any: changing the colour or thickness changes it too.
    let custom =
        app.session.view.draw.pens.iter().position(|p| mode == DrawMode::Pen(p.kind) && p.color == set.color && (p.width - set.width).abs() < 0.01);
    let lasso_on = lasso_selection(&app.session).is_some();
    row.group(RibbonGroup::new("Drawing Tools", crate::tl!("Drawing Tools"), IconId::new("pen").into()), |ui| {
        for (key, label, id, on) in [
            ("select", "Select", "draw.select", mode == DrawMode::Select),
            ("lasso", "Lasso", "draw.lasso", mode == DrawMode::Lasso),
            ("eraser", "Eraser", "draw.eraser", mode == DrawMode::Eraser),
            ("pen", "Pen", "draw.pen", mode == DrawMode::Pen(InkTool::Pen) && custom.is_none()),
            ("pencil", "Pencil", "draw.pencil", mode == DrawMode::Pen(InkTool::Pencil) && custom.is_none()),
            ("highlight", "Highlighter", "draw.highlighter", mode == DrawMode::Pen(InkTool::Highlighter) && custom.is_none()),
        ] {
            ToggleButton::command(CommandRef::with_params(id, json!({})))
                .label(crate::tl!(label))
                .icon(crate::icons::spec(key))
                .arrangement(Arrangement::Stacked)
                .selected(on)
                .show(ui);
        }
        // Pens added with Add Pen; right-click one to remove it.
        let pens = app.session.view.draw.pens.clone();
        for (i, pen) in pens.iter().enumerate() {
            gallery_pen(ui, i, *pen, custom == Some(i));
        }
        menu_button(
            ui,
            Button::new(crate::tl!("Add\nPen")).icon(IconId::new("addPen")).tooltip(crate::tl!("Add Pen")).arrangement(Arrangement::Stacked),
            crate::tl!("Add Pen"),
            Some("draw.addPen"),
            |ui, _| {
                for (label, kind) in [("Pen", InkTool::Pen), ("Pencil", InkTool::Pencil), ("Highlighter", InkTool::Highlighter)] {
                    // A new pen starts as that kind's current colour and thickness.
                    let cur = app.session.view.draw.settings(kind);
                    MenuItem::command(CommandRef::with_params(
                        "draw.addPen",
                        json!({"kind": kind.name(), "color": cur.color.hex(), "width": cur.width}),
                    ))
                    .label(crate::tl!(label))
                    .show(ui);
                }
            },
        );
        // Colour and Thickness show their icons alone, as the old ribbon showed them.
        super::common::icon_only(ui, |ui| {
            Stack::new().gap(Gap::Tight).show(ui, |ui, _| {
                // With ink lasso-selected the colour goes to the selection, else to the pen.
                let target = if lasso_on { "draw.lassoColor" } else { pen_command(tool) };
                let ink = egui::Color32::from_rgb(set.color.0, set.color.1, set.color.2);
                // The main part picks up the pen; the menu sets the colour the bar shows.
                let _ = SplitButton::<String>::command(
                    CommandRef::with_params(pen_command(tool), json!({})),
                    SplitMenu::Body(Box::new(|ui, _| {
                        let _ = crate::colors::menu(ui, app, Target::cmd(target));
                    })),
                )
                .label(crate::tl!("Color"))
                .icon(crate::icons::spec("fontcolor").with_mark(IconMark::bar(DataColor::new(ink))))
                .show(ui);
                menu_button(
                    ui,
                    Button::new(crate::tl!("Thickness")).icon(IconId::new("thickness")).tooltip(crate::tl!("Thickness")),
                    crate::tl!("Thickness"),
                    Some(pen_command(tool)),
                    |ui, _| {
                        let widths: &[f32] =
                            if tool == InkTool::Highlighter { &[4.0, 8.0, 12.0, 18.0, 24.0] } else { &[0.5, 1.0, 1.5, 2.5, 3.5, 5.0] };
                        for w in widths {
                            let label = crate::i18n::fmt(crate::tl!("{n} pt"), &[("n", &w.to_string())]);
                            MenuItem::command(CommandRef::with_params(pen_command(tool), json!({"width": w})))
                                .label(label)
                                .radio_mark((set.width - w).abs() < 0.01)
                                .show(ui);
                        }
                    },
                );
            })
        });
        // The colour or thickness a menu just set goes to the gallery pen that was in use too.
        if !lasso_on {
            update_gallery_pen(app, custom, tool);
        }
    });
    row.group(RibbonGroup::new("Convert", crate::tl!("Convert"), IconId::new("inkToShape").into()), |ui| {
        let on = app.session.view.draw.ink_to_shape;
        ToggleButton::command(CommandRef::with_params("draw.inkToShape", json!({})))
            .label(crate::tl!("Ink to\nShape"))
            .icon(IconId::new("inkToShape"))
            .arrangement(Arrangement::Stacked)
            .selected(on)
            .show(ui);
        Button::command(CommandRef::with_params("draw.inkToMath", json!({})))
            .label(crate::tl!("Ink to\nMath"))
            .icon(crate::icons::spec("inkToMath"))
            .arrangement(Arrangement::Stacked)
            .show(ui);
    });
    row.group(RibbonGroup::new("Insert", crate::tl!("Insert"), IconId::new("canvas").into()), |ui| {
        Button::command(CommandRef::with_params("insert.canvas", json!({})))
            .label(crate::tl!("Drawing\nCanvas"))
            .icon(IconId::new("canvas"))
            .arrangement(Arrangement::Stacked)
            .show(ui);
    });
    row.group(RibbonGroup::new("Replay", crate::tl!("Replay"), IconId::new("replay").into()), |ui| {
        let on = app.session.view.draw.replay;
        ToggleButton::command(CommandRef::with_params("draw.replay", json!({})))
            .label(crate::tl!("Ink\nReplay"))
            .icon(IconId::new("replay"))
            .arrangement(Arrangement::Stacked)
            .selected(on)
            .show(ui);
    });
}

/// The command that picks up `tool`.
fn pen_command(tool: wordcraft_doc::freeform::InkTool) -> &'static str {
    use wordcraft_doc::freeform::InkTool;
    match tool {
        InkTool::Pen => "draw.pen",
        InkTool::Pencil => "draw.pencil",
        InkTool::Highlighter => "draw.highlighter",
    }
}

/// After the colour or thickness menu changed `tool`'s settings, the gallery pen that was in use
/// (`custom`) takes them too.
fn update_gallery_pen(app: &mut WordApp, custom: Option<usize>, tool: wordcraft_doc::freeform::InkTool) {
    let set = app.session.view.draw.settings(tool);
    if let Some(p) = custom.and_then(|i| app.session.view.draw.pens.get_mut(i))
        && p.kind == tool
    {
        p.color = set.color;
        p.width = set.width;
    }
}

/// A pen of the gallery: its kind's icon over a bar of its colour and thickness; a click picks it
/// up, right-click offers Remove Pen.
fn gallery_pen(ui: &mut egui::Ui, i: usize, pen: wordcraft_engine::cmd::draw::CustomPen, active: bool) {
    let (glyph, kind) = match pen.kind {
        InkTool::Pen => ("pen", crate::tl!("Pen")),
        InkTool::Pencil => ("pencil", crate::tl!("Pencil")),
        InkTool::Highlighter => ("highlight", crate::tl!("Highlighter")),
    };
    let color = egui::Color32::from_rgb(pen.color.0, pen.color.1, pen.color.2);
    let bar = (pen.width * 0.6).clamp(1.5, 8.0);
    let size = crate::i18n::fmt(crate::tl!("{n} pt"), &[("n", &pen.width.to_string())]);
    let tip = format!("{kind}: {size}, #{}", pen.color.hex());
    let response = ToggleButton::command(CommandRef::with_params(pen_command(pen.kind), json!({"color": pen.color.hex(), "width": pen.width})))
        .label(kind)
        .icon(crate::icons::spec(glyph).with_mark(IconMark::weighted_bar(DataColor::new(color), bar)))
        .arrangement(Arrangement::Stacked)
        .tooltip(tip)
        .selected(active)
        .show(ui)
        .response;
    Menu::context(crate::tl!("Remove Pen"), &response).show(ui, |ui, _| {
        MenuItem::command(CommandRef::with_params("draw.removePen", json!({"index": i}))).label(crate::tl!("Remove Pen")).show(ui);
    });
}
