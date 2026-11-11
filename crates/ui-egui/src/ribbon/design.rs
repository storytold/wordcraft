use super::common::{mi, mi_text, stack, top_row};
use super::{CONTENT_H, Rect, Sense, Target, Ui, WordApp, big, group, json, menu_button, small, vec2};

/// The Design tab's style set gallery in a box of its own, sized from its thumbnails and clipped
/// to it, with a gap on each side: the Colors and Fonts column can't be drawn over, or cut by,
/// the thumbnails (#638). In a right-to-left interface, the thumbnails lay out from the right
/// (#5750). Returns the box and the space the thumbnails used.
pub(super) fn style_set_box(app: &mut WordApp, ui: &mut Ui) -> (Rect, Rect) {
    ui.add_space(4.0);
    let (gallery, _) = ui.allocate_exact_size(vec2(crate::previews::STYLE_SET_GALLERY_W, CONTENT_H), Sense::hover());
    let mut gui = ui.new_child(egui::UiBuilder::new().max_rect(gallery).layout(crate::rtl::row_layout(ui, egui::Align::Min)));
    gui.set_clip_rect(gallery.intersect(ui.clip_rect()));
    gui.spacing_mut().item_spacing = vec2(1.0, 1.0);
    crate::previews::style_set_gallery(app, &mut gui);
    ui.add_space(8.0);
    (gallery, gui.min_rect())
}

pub(super) fn design(app: &mut WordApp, ui: &mut Ui) {
    group(ui, "Themes", None, app, |ui, app| {
        menu_button(ui, app, crate::widgets::MenuButtonOptions::new("themes", Some("Themes"), "Themes", true, "design.themes"), |ui, app| {
            for (name, ..) in wordcraft_engine::cmd::design::THEMES {
                mi(ui, app, name, "design.theme", json!({"name": name}));
            }
        });
    });
    group(ui, "Document Formatting", None, app, |ui, app| {
        let colors = |ui: &mut Ui, app: &mut WordApp| {
            menu_button(
                ui,
                app,
                crate::widgets::MenuButtonOptions::new("colors", Some("Colors"), "Theme Colors", false, "design.themeColors"),
                |ui, app| {
                    for (name, ..) in wordcraft_engine::cmd::design::THEMES {
                        mi(ui, app, name, "design.themeColors", json!({"name": name}));
                    }
                },
            );
        };
        let fonts = |ui: &mut Ui, app: &mut WordApp| {
            menu_button(
                ui,
                app,
                crate::widgets::MenuButtonOptions::new("fonts", Some("Fonts"), "Theme Fonts", false, "design.themeFonts"),
                |ui, app| {
                    for (name, h, b, _) in wordcraft_engine::cmd::design::THEMES {
                        mi_text(ui, app, &format!("{}: {h} / {b}", tl!(name)), "design.themeFonts", json!({"heading": h, "body": b}));
                    }
                },
            );
        };
        let para_spacing = |ui: &mut Ui, app: &mut WordApp| {
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
        };
        let effects = |ui: &mut Ui, app: &mut WordApp| {
            small(ui, app, "effectsDesign", Some("Effects"), "Effects", crate::widgets::ButtonAction::new("design.effects", json!({}), false));
        };
        let set_default = |ui: &mut Ui, app: &mut WordApp| {
            small(
                ui,
                app,
                "setDefault",
                Some("Set as Default"),
                "Set as Default",
                crate::widgets::ButtonAction::new("design.setDefault", json!({}), false),
            );
        };
        // Measured: one row below 650 px (520 pt), three rows from there.
        if crate::ribbon_fit::rows_for(ui.ctx(), "Design", "Document Formatting", 1) >= 3 {
            style_set_box(app, ui);
            stack(ui, |ui| {
                colors(ui, app);
                fonts(ui, app);
            });
            stack(ui, |ui| {
                para_spacing(ui, app);
                effects(ui, app);
                set_default(ui, app);
            });
        } else {
            top_row(ui, |ui| {
                style_set_box(app, ui);
                colors(ui, app);
                fonts(ui, app);
                para_spacing(ui, app);
                effects(ui, app);
                set_default(ui, app);
            });
        }
    });
    group(ui, "Page Background", None, app, |ui, app| {
        let watermark = |ui: &mut Ui, app: &mut WordApp, big_btn: bool| {
            menu_button(
                ui,
                app,
                crate::widgets::MenuButtonOptions::new("watermark", Some("Watermark"), "Watermark", big_btn, "design.watermark"),
                |ui, app| {
                    for w in ["DRAFT", "CONFIDENTIAL", "DO NOT COPY", "SAMPLE", "ASAP", "URGENT"] {
                        mi(ui, app, w, "design.watermark", json!({"text": w}));
                    }
                    ui.separator();
                    mi(ui, app, "Custom Watermark…", "ui.dialog", json!({"name": "watermark"}));
                    mi(ui, app, "Remove Watermark", "design.watermark", json!({"remove": true}));
                },
            );
        };
        let page_color = |ui: &mut Ui, app: &mut WordApp, big_btn: bool| {
            menu_button(
                ui,
                app,
                crate::widgets::MenuButtonOptions::new(
                    "pageColor",
                    Some(if big_btn { "Page\nColor" } else { "Page Color" }),
                    "Page Color",
                    big_btn,
                    "design.pageColor",
                ),
                |ui, app| {
                    mi(ui, app, "No Color", "design.pageColor", json!({"color": null}));
                    let theme = app.session.doc.settings.theme_colors.clone();
                    crate::colors::menu(ui, app, &theme, Target::cmd("design.pageColor"));
                },
            );
        };
        let page_borders = |ui: &mut Ui, app: &mut WordApp, big_btn: bool| {
            if big_btn {
                big(ui, app, "pageBorders", "Page\nBorders", "design.pageBorders", json!({}), false);
            } else {
                small(
                    ui,
                    app,
                    "pageBorders",
                    Some("Page Borders"),
                    "Page Borders",
                    crate::widgets::ButtonAction::new("design.pageBorders", json!({}), false),
                );
            }
        };
        // Measured: three rows below 750 px (600 pt), one row from there.
        if crate::ribbon_fit::rows_for(ui.ctx(), "Design", "Page Background", 1) >= 3 {
            stack(ui, |ui| {
                crate::widgets::row(ui, |ui| watermark(ui, app, false));
                crate::widgets::row(ui, |ui| page_color(ui, app, false));
                crate::widgets::row(ui, |ui| page_borders(ui, app, false));
            });
        } else {
            top_row(ui, |ui| {
                watermark(ui, app, true);
                page_color(ui, app, true);
                page_borders(ui, app, true);
            });
        }
    });
}
