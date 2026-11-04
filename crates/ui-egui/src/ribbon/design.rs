use super::common::{mi, mi_text, stack};
use super::{CONTENT_H, Rect, Sense, Target, Ui, WordApp, big, group, json, menu_button, small, vec2};

/// The Design tab's style set gallery in a box of its own, sized from its thumbnails and clipped
/// to it, with a gap on each side: the Colors and Fonts column can't be drawn over, or cut by,
/// the thumbnails (#638). Returns the box and the space the thumbnails used.
pub(super) fn style_set_box(app: &mut WordApp, ui: &mut Ui) -> (Rect, Rect) {
    ui.add_space(4.0);
    let (gallery, _) = ui.allocate_exact_size(vec2(crate::previews::STYLE_SET_GALLERY_W, CONTENT_H), Sense::hover());
    let mut gui = ui.new_child(egui::UiBuilder::new().max_rect(gallery).layout(egui::Layout::left_to_right(egui::Align::Min)));
    gui.set_clip_rect(gallery.intersect(ui.clip_rect()));
    gui.spacing_mut().item_spacing = vec2(1.0, 1.0);
    crate::previews::style_set_gallery(app, &mut gui);
    ui.add_space(8.0);
    (gallery, gui.min_rect())
}

/// The Custom section at the top of the Colors, Fonts and Themes galleries (#1869): the person's
/// own sets by name, each applied with `apply`; right-click to Edit… (colours and fonts) or
/// Delete. `kind` is `design.deleteCustom`'s.
fn custom_section(ui: &mut Ui, app: &mut WordApp, kind: &str, names: &[String], apply: &str) {
    if names.is_empty() {
        return;
    }
    ui.label(egui::RichText::new(tl!("Custom")).font(crate::theme::semibold(11.5)));
    for name in names {
        let resp = ui.add(egui::Button::new(name.as_str()).min_size(vec2(200.0, 0.0)));
        if resp.clicked() {
            crate::widgets::run_from_ribbon(app, apply, json!({"name": name}));
            ui.close();
        }
        resp.context_menu(|ui| {
            if kind != "theme" && ui.button(tl!("Edit…")).clicked() {
                let d = match kind {
                    "colors" => crate::dialogs_themes::ColorsForm::edit(app, name).map(crate::dialogs_themes::ThemeDialog::Colors),
                    _ => crate::dialogs_themes::FontsForm::edit(app, name).map(crate::dialogs_themes::ThemeDialog::Fonts),
                };
                if let Some(d) = d {
                    app.dialog = Some(crate::dialogs::Dialog::Themes(Box::new(d)));
                }
                ui.close();
            }
            if ui.button(tl!("Delete")).clicked() {
                crate::widgets::run_from_ribbon(app, "design.deleteCustom", json!({"kind": kind, "name": name}));
                ui.close();
            }
        });
    }
    ui.separator();
}

pub(super) fn design(app: &mut WordApp, ui: &mut Ui) {
    group(ui, "Themes", None, app, |ui, app| {
        menu_button(ui, app, crate::widgets::MenuButtonOptions::new("themes", Some("Themes"), "Themes", true, "design.themes"), |ui, app| {
            // The person's saved themes first, as in Word (#1869).
            let custom: Vec<String> = app.session.custom_themes.themes.iter().map(|t| t.name.clone()).collect();
            custom_section(ui, app, "theme", &custom, "design.theme");
            for (name, ..) in wordcraft_engine::cmd::design::THEMES {
                mi(ui, app, name, "design.theme", json!({"name": name}));
            }
            // The browser's file picker opens what it picks as a document: no theme files there.
            if app.services.open_async.is_none() {
                ui.separator();
                mi(ui, app, "Browse for Themes…", "design.openTheme", json!({}));
                mi(ui, app, "Save Current Theme…", "design.saveTheme", json!({}));
            }
        });
    });
    group(ui, "Document Formatting", None, app, |ui, app| {
        style_set_box(app, ui);
        stack(ui, |ui| {
            menu_button(
                ui,
                app,
                crate::widgets::MenuButtonOptions::new("colors", Some("Colors"), "Theme Colors", false, "design.themeColors"),
                |ui, app| {
                    let custom: Vec<String> = app.session.custom_themes.colors.iter().map(|c| c.name.clone()).collect();
                    custom_section(ui, app, "colors", &custom, "design.themeColors");
                    for (name, ..) in wordcraft_engine::cmd::design::THEMES {
                        mi(ui, app, name, "design.themeColors", json!({"name": name}));
                    }
                    ui.separator();
                    mi(ui, app, "Customize Colors…", "design.customColors", json!({}));
                },
            );
            menu_button(
                ui,
                app,
                crate::widgets::MenuButtonOptions::new("fonts", Some("Fonts"), "Theme Fonts", false, "design.themeFonts"),
                |ui, app| {
                    let custom: Vec<String> = app.session.custom_themes.fonts.iter().map(|c| c.name.clone()).collect();
                    custom_section(ui, app, "fonts", &custom, "design.themeFonts");
                    for (name, h, b, _) in wordcraft_engine::cmd::design::THEMES {
                        mi_text(ui, app, &format!("{}: {h} / {b}", tl!(name)), "design.themeFonts", json!({"heading": h, "body": b}));
                    }
                    ui.separator();
                    mi(ui, app, "Customize Fonts…", "design.customFonts", json!({}));
                },
            );
        });
        stack(ui, |ui| {
            menu_button(
                ui,
                app,
                crate::widgets::MenuButtonOptions::new(
                    "paraSpacing",
                    Some("Paragraph Spacing"),
                    "Paragraph Spacing",
                    false,
                    "design.paragraphSpacing",
                ),
                |ui, app| {
                    for (l, v) in [
                        ("Default", "default"),
                        ("No Paragraph Space", "none"),
                        ("Compact", "compact"),
                        ("Tight", "tight"),
                        ("Open", "open"),
                        ("Relaxed", "relaxed"),
                        ("Double", "double"),
                    ] {
                        // "Open" here is roomy spacing, not the file verb: only a `spacing` row
                        // translates it, else it stays English.
                        if l == "Open" {
                            mi_text(ui, app, crate::i18n::get("spacing", l).unwrap_or(l), "design.paragraphSpacing", json!({"value": v}));
                            continue;
                        }
                        mi(ui, app, l, "design.paragraphSpacing", json!({"value": v}));
                    }
                },
            );
            small(ui, app, "effectsDesign", Some("Effects"), "Effects", crate::widgets::ButtonAction::new("design.effects", json!({}), false));
            small(
                ui,
                app,
                "setDefault",
                Some("Set as Default"),
                "Set as Default",
                crate::widgets::ButtonAction::new("design.setDefault", json!({}), false),
            );
        });
    });
    group(ui, "Page Background", None, app, |ui, app| {
        menu_button(
            ui,
            app,
            crate::widgets::MenuButtonOptions::new("watermark", Some("Watermark"), "Watermark", true, "design.watermark"),
            |ui, app| {
                for w in ["DRAFT", "CONFIDENTIAL", "DO NOT COPY", "SAMPLE", "ASAP", "URGENT"] {
                    mi(ui, app, w, "design.watermark", json!({"text": w}));
                }
                ui.separator();
                mi(ui, app, "Custom Watermark…", "ui.dialog", json!({"name": "watermark"}));
                mi(ui, app, "Remove Watermark", "design.watermark", json!({"remove": true}));
            },
        );
        menu_button(
            ui,
            app,
            crate::widgets::MenuButtonOptions::new("pageColor", Some("Page\nColor"), "Page Color", true, "design.pageColor"),
            |ui, app| {
                mi(ui, app, "No Color", "design.pageColor", json!({"color": null}));
                let theme = app.session.doc.settings.theme_colors.clone();
                crate::colors::menu(ui, app, &theme, Target::cmd("design.pageColor"));
            },
        );
        big(ui, app, "pageBorders", "Page\nBorders", "design.pageBorders", json!({}), false);
    });
}
