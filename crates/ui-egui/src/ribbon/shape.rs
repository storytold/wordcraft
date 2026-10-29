use super::arrange::{align_menu, rotate_menu, rotation_field, wrap_menu};
use super::common::{mi, stack};
use super::language::mi_check;
use super::{Target, Ui, WordApp, big, group, json, menu_button, small, vec2};

/// Shape Fill › Gradient: No Gradient, Light and Dark Variations of the shape's colour, More
/// Gradients… (the Format Shape pane).
pub(crate) fn gradient_menu(ui: &mut Ui, app: &mut WordApp, fill: Option<&wordcraft_doc::fill::Fill>) {
    use wordcraft_engine::cmd::shape_fill;
    let pos = wordcraft_engine::cmd::objects::selected(&app.session).map(|(p, _)| p);
    let main = match (fill, &pos) {
        (Some(f), Some(p)) => crate::format_shape::variation_base(ui.ctx(), p, f),
        (Some(f), None) => f.main_color(),
        _ => app.session.doc.settings.theme_colors.get(4).copied().unwrap_or(wordcraft_doc::Rgb(68, 114, 196)),
    };
    // No Gradient: the shape's colour, solid.
    ui.label(egui::RichText::new(tl!("No Gradient")).small().weak());
    if crate::format_shape::swatch(ui, &wordcraft_doc::fill::Fill::solid(main), "No Gradient") {
        let _ = app.run("shape.fill", json!({"color": main.hex()}));
        ui.close();
    }
    for (title, tiles) in ["Light Variations", "Dark Variations"].into_iter().zip(shape_fill::variations(main)) {
        ui.label(egui::RichText::new(tl!(title)).small().weak());
        egui::Grid::new(title).spacing(vec2(4.0, 4.0)).show(ui, |ui| {
            for (i, (f, name)) in tiles.iter().enumerate() {
                if crate::format_shape::swatch(ui, f, name) {
                    let _ = app.run("shape.fill", shape_fill::params(Some(f)));
                    if let (Some(p), Some(Some(now))) = (pos.clone(), crate::format_shape::current(app)) {
                        crate::format_shape::remember_variation(ui.ctx(), p, now, main);
                    }
                    ui.close();
                }
                if i % 5 == 4 {
                    ui.end_row();
                }
            }
        });
    }
    ui.separator();
    if ui.button(tl!("More Gradients…")).clicked() {
        let _ = app.run("view.formatShapePane", json!({"value": true}));
        ui.close();
    }
}

/// Shape Format: fill, outline and effects of the selected shape, and its arrangement.
pub(super) fn shape_format(app: &mut WordApp, ui: &mut Ui) {
    let (stroke, glow_size, fill) = match wordcraft_engine::cmd::objects::selected(&app.session) {
        Some((_, wordcraft_doc::para::InlineObject::Shape { stroke, effects, fill, .. })) => (stroke, effects.glow.map(|g| g.size), fill),
        Some((_, o)) => (None, None, o.canvas_style().and_then(|c| c.fill.clone())),
        _ => (None, None, None),
    };
    let theme = app.session.doc.settings.theme_colors.clone();
    group(ui, "Shape Styles", None, app, |ui, app| {
        stack(ui, |ui| {
            menu_button(
                ui,
                app,
                crate::widgets::MenuButtonOptions::new("shading", Some("Shape Fill"), "Shape Fill", false, "shape.fill"),
                |ui, app| {
                    mi(ui, app, "No Color", "shape.fill", json!({"color": null}));
                    crate::colors::menu(ui, app, &theme, Target::cmd("shape.fill"));
                    ui.separator();
                    ui.menu_button(tl!("Gradient"), |ui| gradient_menu(ui, app, fill.as_ref()));
                },
            );
            menu_button(
                ui,
                app,
                crate::widgets::MenuButtonOptions::new("pen", Some("Shape Outline"), "Shape Outline", false, "shape.outline"),
                |ui, app| {
                    mi(ui, app, "No Color", "shape.outline", json!({"color": null}));
                    crate::colors::menu(ui, app, &theme, Target::cmd("shape.outline"));
                    ui.separator();
                    ui.menu_button(tl!("Weight"), |ui| {
                        let color = stroke.unwrap_or(wordcraft_doc::Rgb::BLACK).hex();
                        for w in [0.25, 0.5, 0.75, 1.0, 1.5, 2.25, 3.0, 4.5, 6.0] {
                            mi(ui, app, &format!("{w} pt"), "shape.outline", json!({"color": color, "width": w}));
                        }
                    });
                },
            );
            menu_button(
                ui,
                app,
                crate::widgets::MenuButtonOptions::new("shapeEffects", Some("Shape Effects"), "Shadow, glow and soft edges", false, "shape.effects"),
                |ui, app| shape_effects_menu(ui, app, &theme, glow_size),
            );
        });
    });
    alt_text_group(ui, app);
    if wordcraft_engine::cmd::canvas::selected_canvas(&app.session).is_some() {
        group(ui, "Drawing Canvas", None, app, |ui, app| {
            menu_button(ui, app, crate::widgets::MenuButtonOptions::new("canvas", Some("Drawing\nCanvas"), "Drawing Canvas", true, ""), |ui, app| {
                mi(ui, app, "Fit", "canvas.fit", json!({}));
                mi(ui, app, "Expand", "canvas.expand", json!({}));
                ui.menu_button(tl!("Scale Drawing"), |ui| {
                    for pct in [50, 75, 125, 150, 200] {
                        mi(ui, app, &format!("{pct}%"), "canvas.scale", json!({"factor": pct as f32 / 100.0}));
                    }
                });
            });
        });
    }
    group(ui, "WordArt Styles", None, app, |ui, app| {
        menu_button(ui, app, crate::widgets::MenuButtonOptions::new("wordArt", Some("Styles"), "WordArt Styles", true, ""), |ui, app| {
            crate::wordart_menus::style_gallery(ui, app, "wordArt.style")
        });
        stack(ui, |ui| {
            menu_button(ui, app, crate::widgets::MenuButtonOptions::new("textFill", Some("Text Fill"), "Text Fill", false, ""), |ui, app| {
                crate::wordart_menus::fill_menu(ui, app, &theme)
            });
            let outline = crate::wordart_menus::current_outline(app);
            menu_button(
                ui,
                app,
                crate::widgets::MenuButtonOptions::new("textOutline", Some("Text Outline"), "Text Outline", false, ""),
                |ui, app| crate::wordart_menus::outline_menu(ui, app, &theme, outline),
            );
            menu_button(ui, app, crate::widgets::MenuButtonOptions::new("transform", Some("Text Effects"), "Text Effects", false, ""), |ui, app| {
                crate::wordart_menus::effects_menu(ui, app, &theme, true)
            });
        });
    });
    shape_text_group(app, ui);
    group(ui, "Arrange", None, app, |ui, app| {
        big(ui, app, "position", "Position", "arrange.position", json!({}), false);
        wrap_menu(ui, app);
        stack(ui, |ui| {
            rotate_menu(ui, app, Some("Rotate"));
            align_menu(ui, app, Some("Align"));
        });
    });
    group(ui, "Size", None, app, |ui, app| {
        let (w0, h0) =
            wordcraft_engine::cmd::objects::selected(&app.session).and_then(|(_, o)| o.frame().map(|(w, h, _)| (w, h))).unwrap_or((0.0, 0.0));
        stack(ui, |ui| {
            crate::widgets::row(ui, |ui| {
                for (label, key, v0) in [("W:", "width", w0), ("H:", "height", h0)] {
                    let label = ui.label(egui::RichText::new(tl!(label)).small());
                    let mut v = v0;
                    // `picture.size`'s own range; a size outside it (a thin line) is shown as it
                    // is (clamped, egui would report a change and resize the shape unasked).
                    let r = ui.add(egui::DragValue::new(&mut v).speed(1.0).range(1.0..=4000.0).clamp_existing_to_range(false).suffix(" pt"));
                    crate::widgets::labelled_by(&r, &[&label]);
                    if r.changed() && v != v0 {
                        if r.dragged() && !r.drag_started() {
                            app.session.join_next_undo();
                        }
                        let _ = app.run("picture.size", json!({key: v, "lockAspect": false}));
                    }
                }
            });
            ui.add_space(2.0);
            crate::widgets::row(ui, |ui| rotation_field(ui, app));
        });
    });
}

/// Shape Format › Text: the text box's Text Direction and Align Text, and Create Link (Break Link
/// once it is linked to another box).
fn shape_text_group(app: &mut WordApp, ui: &mut Ui) {
    use wordcraft_doc::props::{TextVert, VAlign};
    let target = wordcraft_engine::cmd::objects::text_box_target(&app.session);
    let body = target.and_then(|(_, id)| app.session.doc.parts.get(&id).map(|p| p.body)).unwrap_or_default();
    group(ui, "Text", None, app, |ui, app| {
        stack(ui, |ui| {
            menu_button(
                ui,
                app,
                crate::widgets::MenuButtonOptions::new("textDirection", Some("Text Direction"), "Text Direction", false, ""),
                |ui, app| {
                    for (label, vert) in [
                        ("Horizontal", TextVert::Horz),
                        ("Rotate all text 90°", TextVert::Vert),
                        ("Rotate all text 270°", TextVert::Vert270),
                        ("Stacked", TextVert::Stacked),
                    ] {
                        mi_check(ui, app, label, body.vert == vert, "shape.textDirection", json!({"dir": vert.ooxml()}));
                    }
                },
            );
            menu_button(ui, app, crate::widgets::MenuButtonOptions::new("alignText", Some("Align Text"), "Align Text", false, ""), |ui, app| {
                for (label, anchor) in [("Top", VAlign::Top), ("Middle", VAlign::Center), ("Bottom", VAlign::Bottom)] {
                    mi_check(ui, app, label, body.anchor == anchor, "shape.alignText", json!({"anchor": wordcraft_doc::props::anchor_ooxml(anchor)}));
                }
            });
            if body.next.is_some() {
                small(
                    ui,
                    app,
                    "textBoxUnlink",
                    Some("Break Link"),
                    "Break Link",
                    crate::widgets::ButtonAction::new("shape.breakLink", json!({}), false),
                );
            } else {
                let waiting = app.canvas.link_from.is_some();
                small(
                    ui,
                    app,
                    "textBoxLink",
                    Some("Create Link"),
                    "Create Link",
                    crate::widgets::ButtonAction::toggle("ui.linkTextBox", json!({}), waiting),
                );
            }
        });
    });
}

/// Picture/Shape Format › Accessibility: Alt Text opens the Alt Text pane.
pub(super) fn alt_text_group(ui: &mut Ui, app: &mut WordApp) {
    group(ui, "Accessibility", None, app, |ui, app| {
        big(ui, app, "altText", "Alt\nText", "view.altTextPane", json!({"value": true}), false);
    });
}

/// Shape Format › Shape Effects: Shadow, Glow and Soft Edges, each with presets and a "No …" entry.
fn shape_effects_menu(ui: &mut Ui, app: &mut WordApp, theme: &[wordcraft_doc::Rgb], glow_size: Option<f32>) {
    ui.menu_button(tl!("Shadow"), |ui| {
        mi(ui, app, "No Shadow", "shape.effects", json!({"shadow": null}));
        ui.separator();
        for (id, label) in wordcraft_doc::effects::SHADOW_PRESETS {
            mi(ui, app, label, "shape.effects", json!({"shadow": id}));
        }
    });
    ui.menu_button(tl!("Glow"), |ui| {
        mi(ui, app, "No Glow", "shape.effects", json!({"glow": null}));
        ui.separator();
        for size in [5.0, 8.0, 11.0, 18.0] {
            mi(ui, app, &format!("{size} pt"), "shape.effects", json!({"glow": size}));
        }
        ui.separator();
        ui.menu_button(tl!("Glow Colors"), |ui| {
            let target = Target::at("shape.effects", json!({"glow": {"color": null, "size": glow_size.unwrap_or(8.0)}}), "/glow/color");
            crate::colors::menu(ui, app, theme, target);
        });
    });
    ui.menu_button(tl!("Soft Edges"), |ui| {
        mi(ui, app, "No Soft Edges", "shape.effects", json!({"softEdge": null}));
        ui.separator();
        for r in [1.0, 2.5, 5.0, 10.0, 25.0, 50.0] {
            mi(ui, app, &format!("{r} pt"), "shape.effects", json!({"softEdge": r}));
        }
    });
}
