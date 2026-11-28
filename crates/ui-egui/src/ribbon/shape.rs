use craft::behavior::{ColumnWidth, IconId};
use craft::draw::ui::gradient::GradientSwatch;
use craft::menu::ribbon::{RibbonGroup, RibbonRow};
use craft::ui::buttons::{Arrangement, Button, ToggleButton};
use craft::ui::contract::{Gap, Props as _, Show as _, ShowContainer as _};
use craft::ui::display::Divider;
use craft::ui::layout_components::{Grid, Row, Stack};
use craft::ui::menu::{Menu, MenuItem};
use craft::ui::runtime::commands::CommandRef;
use craft::ui::runtime::overlay::close_popups;

use super::arrange::{align_menu, rotate_menu, rotation_field, wrap_menu};
use super::common::{menu_button, size_field, weight_menu};
use super::{Ui, WordApp, json};
use crate::colors::Target;

/// The variation grids' columns: five tiles per row, each as wide as its tile.
const VARIATION_GRID: [ColumnWidth; 5] = [ColumnWidth::Content; 5];

/// Shape Fill › Gradient: No Gradient, Light and Dark Variations of the shape's colour, More
/// Gradients… (the Format Shape pane). A pick closes the menus, as a colour pick does.
pub(crate) fn gradient_menu(ui: &mut Ui, app: &mut WordApp, fill: Option<&wordcraft_doc::fill::Fill>) {
    use wordcraft_engine::cmd::shape_fill;
    let pos = wordcraft_engine::cmd::objects::selected(&app.session).map(|(p, _)| p);
    let main = match (fill, &pos) {
        (Some(f), Some(p)) => crate::format_shape::variation_base(app, p, f),
        (Some(f), None) => f.main_color(),
        _ => app.session.doc.settings.theme_colors.get(4).copied().unwrap_or(wordcraft_doc::Rgb(68, 114, 196)),
    };
    // No Gradient: the shape's colour, solid.
    MenuItem::heading(tl!("No Gradient")).show(ui);
    let solid = wordcraft_doc::fill::Fill::solid(main);
    let (stops, tile) = crate::format_shape::fill_tile(&solid);
    if GradientSwatch::new(tl!("No Gradient"), &stops).tile(tile).show(ui).clicked {
        let _ = app.run("shape.fill", json!({"color": main.hex()}));
        close_popups(ui.ctx());
    }
    for (title, tiles) in ["Light Variations", "Dark Variations"].into_iter().zip(shape_fill::variations(main)) {
        MenuItem::heading(tl!(title)).show(ui);
        Grid::new(&VARIATION_GRID).show(ui, |ui, _| {
            for (f, name) in tiles.iter() {
                let (stops, tile) = crate::format_shape::fill_tile(f);
                if GradientSwatch::new(name.clone(), &stops).tile(tile).show(ui).clicked {
                    let _ = app.run("shape.fill", shape_fill::params(Some(f)));
                    if let (Some(p), Some(Some(now))) = (pos.clone(), crate::format_shape::current(app)) {
                        crate::format_shape::remember_variation(app, p, now, main);
                    }
                    close_popups(ui.ctx());
                }
            }
        });
    }
    Divider::horizontal().show(ui);
    MenuItem::command(CommandRef::with_params("view.formatShapePane", json!({"value": true}))).label(tl!("More Gradients…")).show(ui);
}

/// The outline (colour, width) Shape Outline changes: the selected shape's, or the selected
/// drawing canvas's border.
pub(super) fn selected_outline(s: &wordcraft_engine::Session) -> Option<(wordcraft_doc::Rgb, f32)> {
    match wordcraft_engine::cmd::objects::selected(s)?.1 {
        wordcraft_doc::para::InlineObject::Shape { stroke, stroke_width, .. } => stroke.map(|c| (c, stroke_width)),
        o => o.canvas_style().and_then(|c| c.stroke.map(|stroke| (stroke, c.stroke_width))),
    }
}

/// Shape Outline › Weight: a new weight keeps the outline's colour (black when it has none).
pub(super) fn shape_weight_menu(ui: &mut Ui, app: &mut WordApp) {
    let outline = selected_outline(&app.session);
    let color = outline.map_or(wordcraft_doc::Rgb::BLACK, |o| o.0).hex();
    weight_menu(ui, app, "shape.outline", outline.map(|o| o.1), |w| json!({"color": color, "width": w}));
}

/// Shape Format: fill, outline and effects of the selected shape, and its arrangement.
pub(super) fn shape_format(row: &mut RibbonRow<'_>, app: &mut WordApp) {
    let (glow_size, fill) = match wordcraft_engine::cmd::objects::selected(&app.session) {
        Some((_, wordcraft_doc::para::InlineObject::Shape { effects, fill, .. })) => (effects.glow.map(|g| g.size), fill),
        Some((_, o)) => (None, o.canvas_style().and_then(|c| c.fill.clone())),
        _ => (None, None),
    };
    let theme = app.session.doc.settings.theme_colors.clone();
    row.group(RibbonGroup::new("shape.styles", tl!("Shape Styles"), IconId::new("wordcraft.more").into()), |ui| {
        Stack::new().gap(Gap::Tight).show(ui, |ui, _| {
            menu_button(ui, Button::new(tl!("Shape Fill")).icon(IconId::new("shading")), tl!("Shape Fill"), Some("shape.fill"), |ui, _| {
                MenuItem::command(CommandRef::with_params("shape.fill", json!({"color": null}))).label(tl!("No Color")).show(ui);
                crate::colors::menu(ui, app, Target::cmd("shape.fill"));
                Divider::horizontal().show(ui);
                Menu::submenu_row(ui, tl!("Gradient"), |ui, _| gradient_menu(ui, app, fill.as_ref()));
            });
            menu_button(ui, Button::new(tl!("Shape Outline")).icon(IconId::new("pen")), tl!("Shape Outline"), Some("shape.outline"), |ui, _| {
                MenuItem::command(CommandRef::with_params("shape.outline", json!({"color": null}))).label(tl!("No Color")).show(ui);
                crate::colors::menu(ui, app, Target::cmd("shape.outline"));
                Divider::horizontal().show(ui);
                Menu::submenu_row(ui, tl!("Weight"), |ui, _| shape_weight_menu(ui, app));
            });
            let effects = Button::new(tl!("Shape Effects")).icon(IconId::new("shapeEffects")).tooltip(tl!("Shadow, glow and soft edges"));
            menu_button(ui, effects, tl!("Shape Effects"), Some("shape.effects"), |ui, _| shape_effects_menu(ui, app, glow_size));
        });
    });
    alt_text_group(row);
    if wordcraft_engine::cmd::canvas::selected_canvas(&app.session).is_some() {
        row.group(RibbonGroup::new("shape.canvas", tl!("Drawing Canvas"), IconId::new("wordcraft.more").into()), |ui| {
            let face =
                Button::new(tl!("Drawing\nCanvas")).icon(IconId::new("canvas")).tooltip(tl!("Drawing Canvas")).arrangement(Arrangement::Stacked);
            menu_button(ui, face, tl!("Drawing Canvas"), None, |ui, _| {
                MenuItem::command(CommandRef::new("canvas.fit")).label(tl!("Fit")).show(ui);
                MenuItem::command(CommandRef::new("canvas.expand")).label(tl!("Expand")).show(ui);
                Menu::submenu_row(ui, tl!("Scale Drawing"), |ui, _| {
                    for pct in [50, 75, 125, 150, 200] {
                        let label = format!("{pct}%");
                        MenuItem::command(CommandRef::with_params("canvas.scale", json!({"factor": pct as f32 / 100.0})))
                            .label(tl!(label.as_str()).to_string())
                            .show(ui);
                    }
                });
            });
        });
    }
    row.group(RibbonGroup::new("shape.wordart", tl!("Decorative Text Styles"), IconId::new("wordcraft.more").into()), |ui| {
        let styles =
            Button::new(tl!("Styles")).icon(crate::icons::spec("wordArt")).tooltip(tl!("Decorative Text Styles")).arrangement(Arrangement::Stacked);
        menu_button(ui, styles, tl!("Decorative Text Styles"), None, |ui, cx| super::galleries::wordart(ui, cx, app, "wordArt.style"));
        let outline = crate::wordart_menus::current_outline(app);
        Stack::new().gap(Gap::Tight).show(ui, |ui, _| {
            menu_button(ui, Button::new(tl!("Text Fill")).icon(crate::icons::spec("textFill")), tl!("Text Fill"), None, |ui, cx| {
                crate::wordart_menus::fill_menu(ui, cx, app, &theme)
            });
            menu_button(ui, Button::new(tl!("Text Outline")).icon(crate::icons::spec("textOutline")), tl!("Text Outline"), None, |ui, _| {
                crate::wordart_menus::outline_menu(ui, app, outline)
            });
            menu_button(ui, Button::new(tl!("Text Effects")).icon(crate::icons::spec("transform")), tl!("Text Effects"), None, |ui, _| {
                crate::wordart_menus::effects_menu(ui, app, true)
            });
        });
    });
    shape_text_group(row, app);
    row.group(RibbonGroup::new("shape.arrange", tl!("Arrange"), IconId::new("wordcraft.more").into()), |ui| {
        Button::command(CommandRef::new("arrange.position")).icon(IconId::new("position")).arrangement(Arrangement::Stacked).show(ui);
        wrap_menu(ui, app);
        Stack::new().gap(Gap::Tight).show(ui, |ui, _| {
            rotate_menu(ui);
            align_menu(ui);
        });
    });
    row.group(RibbonGroup::new("shape.size", tl!("Size"), IconId::new("wordcraft.more").into()), |ui| {
        let (w0, h0) =
            wordcraft_engine::cmd::objects::selected(&app.session).and_then(|(_, o)| o.frame().map(|(w, h, _)| (w, h))).unwrap_or((0.0, 0.0));
        // As Picture Format › Size, each side on its own.
        Stack::new().gap(Gap::Tight).show(ui, |ui, _| {
            Row::new().show(ui, |ui, _| {
                size_field(ui, app, "W:", "width", w0, false, 1.0..=4000.0);
                size_field(ui, app, "H:", "height", h0, false, 1.0..=4000.0);
            });
            Row::new().show(ui, |ui, _| rotation_field(ui, app));
        });
    });
}

/// Shape Format › Text: the text box's Text Direction and Align Text, and Create Link (Break Link
/// once it is linked to another box).
fn shape_text_group(row: &mut RibbonRow<'_>, app: &mut WordApp) {
    use wordcraft_doc::props::{TextVert, VAlign};
    let target = wordcraft_engine::cmd::objects::text_box_target(&app.session);
    let body = target.and_then(|(_, id)| app.session.doc.parts.get(&id).map(|p| p.body)).unwrap_or_default();
    row.group(RibbonGroup::new("shape.text", tl!("Text"), IconId::new("wordcraft.more").into()), |ui| {
        Stack::new().gap(Gap::Tight).show(ui, |ui, _| {
            menu_button(ui, Button::new(tl!("Text Direction")).icon(crate::icons::spec("textDirection")), tl!("Text Direction"), None, |ui, _| {
                for (label, vert) in [
                    ("Horizontal", TextVert::Horz),
                    ("Rotate all text 90°", TextVert::Vert),
                    ("Rotate all text 270°", TextVert::Vert270),
                    ("Stacked", TextVert::Stacked),
                ] {
                    MenuItem::command(CommandRef::with_params("shape.textDirection", json!({"dir": vert.ooxml()})))
                        .label(tl!(label))
                        .radio_mark(body.vert == vert)
                        .show(ui);
                }
            });
            menu_button(ui, Button::new(tl!("Align Text")).icon(IconId::new("alignText")), tl!("Align Text"), None, |ui, _| {
                for (label, anchor) in [("Top", VAlign::Top), ("Middle", VAlign::Center), ("Bottom", VAlign::Bottom)] {
                    MenuItem::command(CommandRef::with_params("shape.alignText", json!({"anchor": wordcraft_doc::props::anchor_ooxml(anchor)})))
                        .label(tl!(label))
                        .radio_mark(body.anchor == anchor)
                        .show(ui);
                }
            });
            if body.next.is_some() {
                Button::command(CommandRef::new("shape.breakLink")).label(tl!("Break Link")).icon(IconId::new("textBoxUnlink")).show(ui);
            } else {
                ToggleButton::command(CommandRef::new("ui.linkTextBox"))
                    .label(tl!("Create Link"))
                    .icon(IconId::new("textBoxLink"))
                    .selected(app.canvas.link_from.is_some())
                    .show(ui);
            }
        });
    });
}

/// Picture/Shape Format › Accessibility: Alt Text opens the Alt Text pane.
pub(super) fn alt_text_group(row: &mut RibbonRow<'_>) {
    row.group(RibbonGroup::new("accessibility", tl!("Accessibility"), IconId::new("wordcraft.more").into()), |ui| {
        Button::command(CommandRef::with_params("view.altTextPane", json!({"value": true})))
            .label(tl!("Alt\nText"))
            .icon(IconId::new("altText"))
            .arrangement(Arrangement::Stacked)
            .show(ui);
    });
}

/// Shape Format › Shape Effects: Shadow, Glow and Soft Edges, each with presets and a "No …" entry.
fn shape_effects_menu(ui: &mut Ui, app: &mut WordApp, glow_size: Option<f32>) {
    Menu::submenu_row(ui, tl!("Shadow"), |ui, _| {
        MenuItem::command(CommandRef::with_params("shape.effects", json!({"shadow": null}))).label(tl!("No Shadow")).show(ui);
        Divider::horizontal().show(ui);
        for (id, label) in wordcraft_doc::effects::SHADOW_PRESETS {
            MenuItem::command(CommandRef::with_params("shape.effects", json!({"shadow": id}))).label(tl!(label)).show(ui);
        }
    });
    Menu::submenu_row(ui, tl!("Glow"), |ui, _| {
        MenuItem::command(CommandRef::with_params("shape.effects", json!({"glow": null}))).label(tl!("No Glow")).show(ui);
        Divider::horizontal().show(ui);
        for size in [5.0, 8.0, 11.0, 18.0] {
            let label = format!("{size} pt");
            MenuItem::command(CommandRef::with_params("shape.effects", json!({"glow": size}))).label(tl!(label.as_str()).to_string()).show(ui);
        }
        Divider::horizontal().show(ui);
        Menu::submenu_row(ui, tl!("Glow Colors"), |ui, _| {
            let target = Target::at("shape.effects", json!({"glow": {"color": null, "size": glow_size.unwrap_or(8.0)}}), "/glow/color");
            crate::colors::menu(ui, app, target);
        });
    });
    Menu::submenu_row(ui, tl!("Soft Edges"), |ui, _| {
        MenuItem::command(CommandRef::with_params("shape.effects", json!({"softEdge": null}))).label(tl!("No Soft Edges")).show(ui);
        Divider::horizontal().show(ui);
        for r in [1.0, 2.5, 5.0, 10.0, 25.0, 50.0] {
            let label = format!("{r} pt");
            MenuItem::command(CommandRef::with_params("shape.effects", json!({"softEdge": r}))).label(tl!(label.as_str()).to_string()).show(ui);
        }
    });
}
