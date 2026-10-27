use super::common::{mi, mi_text, stack};
use super::*;

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

pub(super) fn design(app: &mut WordApp, ui: &mut Ui) {
    group(ui, "Themes", None, app, |ui, app| {
        menu_button(ui, app, "themes", Some("Themes"), "Themes", true, "design.themes", |ui, app| {
            for (name, ..) in wordcraft_engine::cmd::design::THEMES {
                mi(ui, app, name, "design.theme", json!({"name": name}));
            }
        });
    });
    group(ui, "Document Formatting", None, app, |ui, app| {
        style_set_box(app, ui);
        stack(ui, |ui| {
            menu_button(ui, app, "colors", Some("Colors"), "Theme Colors", false, "design.themeColors", |ui, app| {
                for (name, ..) in wordcraft_engine::cmd::design::THEMES {
                    mi(ui, app, name, "design.themeColors", json!({"name": name}));
                }
            });
            menu_button(ui, app, "fonts", Some("Fonts"), "Theme Fonts", false, "design.themeFonts", |ui, app| {
                for (name, h, b, _) in wordcraft_engine::cmd::design::THEMES {
                    mi_text(ui, app, &format!("{}: {h} / {b}", tl!(name)), "design.themeFonts", json!({"heading": h, "body": b}));
                }
            });
        });
        stack(ui, |ui| {
            menu_button(ui, app, "paraSpacing", Some("Paragraph Spacing"), "Paragraph Spacing", false, "design.paragraphSpacing", |ui, app| {
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
            });
            small(ui, app, "effectsDesign", Some("Effects"), "Effects", "design.effects", json!({}), false);
            small(ui, app, "setDefault", Some("Set as Default"), "Set as Default", "design.setDefault", json!({}), false);
        });
    });
    group(ui, "Page Background", None, app, |ui, app| {
        menu_button(ui, app, "watermark", Some("Watermark"), "Watermark", true, "design.watermark", |ui, app| {
            for w in ["DRAFT", "CONFIDENTIAL", "DO NOT COPY", "SAMPLE", "ASAP", "URGENT"] {
                mi(ui, app, w, "design.watermark", json!({"text": w}));
            }
            ui.separator();
            mi(ui, app, "Custom Watermark…", "ui.dialog", json!({"name": "watermark"}));
            mi(ui, app, "Remove Watermark", "design.watermark", json!({"remove": true}));
        });
        menu_button(ui, app, "pageColor", Some("Page\nColor"), "Page Color", true, "design.pageColor", |ui, app| {
            mi(ui, app, "No Color", "design.pageColor", json!({"color": null}));
            let theme = app.session.doc.settings.theme_colors.clone();
            crate::colors::menu(ui, app, &theme, Target::cmd("design.pageColor"));
        });
        big(ui, app, "pageBorders", "Page\nBorders", "design.pageBorders", json!({}), false);
    });
}
