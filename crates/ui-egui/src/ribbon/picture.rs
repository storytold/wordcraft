use super::arrange::{align_menu, group_menu, rotate_menu, rotation_field, wrap_menu};
use super::common::{esc_cancelled, mi, stack};
use super::shape::alt_text_group;
use super::{MeasureExt, Target, Ui, WordApp, big, group, json, menu_button, small};

/// Draws the Picture Format tab's Adjust, Picture Styles, Accessibility (Alt Text), Arrange and
/// Size groups.
pub(super) fn picture_format(app: &mut WordApp, ui: &mut Ui) {
    group(ui, "Adjust", None, app, |ui, app| {
        menu_button(
            ui,
            app,
            crate::widgets::MenuButtonOptions::new("picture", Some("Corrections"), "Brightness, contrast and sharpness", true, "picture.corrections"),
            |ui, app| {
                for (l, b, c) in
                    [("Brighter +20%", 20.0, 0.0), ("Darker −20%", -20.0, 0.0), ("More Contrast +20%", 0.0, 20.0), ("Less Contrast −20%", 0.0, -20.0)]
                {
                    mi(ui, app, l, "picture.corrections", json!({"brightness": b, "contrast": c}));
                }
                ui.separator();
                mi(ui, app, "Sharpen", "picture.corrections", json!({"sharpen": 40.0}));
                mi(ui, app, "Soften", "picture.corrections", json!({"sharpen": -40.0}));
            },
        );
        menu_button(
            ui,
            app,
            crate::widgets::MenuButtonOptions::new("picture", Some("Color"), "Saturation and recolor", true, "picture.color"),
            |ui, app| {
                for (l, v) in [("Full Saturation 100%", 100.0), ("Muted 50%", 50.0), ("Gray 0%", 0.0), ("Vivid 200%", 200.0)] {
                    mi(ui, app, l, "picture.color", json!({"mode": "saturation", "saturation": v}));
                }
                ui.separator();
                mi(ui, app, "Grayscale", "picture.color", json!({"mode": "grayscale"}));
                mi(ui, app, "Sepia", "picture.color", json!({"mode": "sepia"}));
                mi(ui, app, "Washout", "picture.color", json!({"mode": "washout"}));
            },
        );
        menu_button(
            ui,
            app,
            crate::widgets::MenuButtonOptions::new(
                "icons",
                Some("Icon Color"),
                "Recolor a one-color picture, such as an icon",
                true,
                "picture.iconColor",
            ),
            |ui, app| {
                let theme = app.session.doc.settings.theme_colors.clone();
                crate::colors::menu(ui, app, &theme, Target::cmd("picture.iconColor"));
            },
        );
        menu_button(
            ui,
            app,
            crate::widgets::MenuButtonOptions::new("picture", Some("Transparency"), "Picture transparency", true, "picture.transparency"),
            |ui, app| {
                for v in [0.0, 15.0, 30.0, 50.0, 65.0, 80.0, 95.0] {
                    mi(ui, app, &format!("{v:.0}%"), "picture.transparency", json!({"percent": v}));
                }
                ui.separator();
                // An exact value, dragged with a live preview: absolute from the original
                // pixels, one undo step for the whole drag (same pattern as Size › W/H).
                // The current value isn't stored anywhere, so the slider reopens where it
                // was last left for this picture.
                let (media, off) = match wordcraft_engine::cmd::objects::selected(&app.session) {
                    Some((pos, wordcraft_doc::para::InlineObject::Image { media, .. })) => (media, pos.off),
                    _ => (String::new(), 0),
                };
                let key = egui::Id::unique(("picture_transparency", media, off));
                let mut v = ui.data(|d| d.get_temp::<f64>(key)).unwrap_or(0.0);
                let r = ui.add(egui::DragValue::new(&mut v).speed(1.0).range(0.0..=100.0).suffix("%"));
                if r.changed() {
                    ui.data_mut(|d| d.insert_temp(key, v));
                    if r.dragged() && !r.drag_started() {
                        app.session.join_next_undo();
                    }
                    let _ = app.run("picture.transparency", json!({"percent": v}));
                }
            },
        );
        stack(ui, |ui| {
            small(ui, app, "picture", Some("Change"), "Change Picture", crate::widgets::ButtonAction::new("ui.changePicture", json!({}), false));
            small(ui, app, "picture", Some("Reset"), "Reset Picture", crate::widgets::ButtonAction::new("picture.reset", json!({}), false));
        });
    });
    group(ui, "Picture Styles", None, app, |ui, app| {
        menu_button(
            ui,
            app,
            crate::widgets::MenuButtonOptions::new("picture", Some("Styles"), "Frames and effects", true, "picture.style"),
            |ui, app| {
                for (l, s) in [
                    ("Simple Frame", "simpleFrame"),
                    ("Thick Frame", "thickFrame"),
                    ("Rounded", "rounded"),
                    ("Soft Edge", "softEdge"),
                    ("Shadow", "shadow"),
                ] {
                    mi(ui, app, l, "picture.style", json!({"style": s}));
                }
            },
        );
        menu_button(
            ui,
            app,
            crate::widgets::MenuButtonOptions::new("picture", Some("Border"), "Picture border", true, "picture.border"),
            |ui, app| {
                for (l, w) in [("Thin", 2), ("Medium", 4), ("Thick", 8)] {
                    mi(ui, app, l, "picture.border", json!({"width": w}));
                }
            },
        );
    });
    alt_text_group(ui, app);
    group(ui, "Arrange", None, app, |ui, app| {
        big(ui, app, "position", "Position", "arrange.position", json!({}), false);
        wrap_menu(ui, app);
        stack(ui, |ui| {
            rotate_menu(ui, app, Some("Rotate"));
            align_menu(ui, app, Some("Align"));
            group_menu(ui, app, Some("Group"));
        });
    });
    // The launcher opens the Size dialog: scale, Lock aspect ratio, the original size (#2573).
    group(ui, "Size", Some("picture.size"), app, |ui, app| {
        let (media, off, w0, h0, crop0, alt0) = match wordcraft_engine::cmd::objects::selected(&app.session) {
            Some((pos, wordcraft_doc::para::InlineObject::Image { media, w, h, alt, crop, .. })) => (media, pos.off, w, h, crop, alt),
            _ => (String::new(), 0, 0.0, 0.0, [0.0; 4], String::new()),
        };
        // `picture.size` locks the aspect ratio unless told otherwise, so one dimension suffices.
        // The fields show the interface unit and take a size typed in any unit, pixels too
        // (`300 px`, #1901), applied once typed (Enter, Tab or a click elsewhere; Esc cancels),
        // never per keystroke; a drag applies as it goes, one undo step.
        let unit = crate::options::unit();
        let k = unit.pt_per_unit();
        stack(ui, |ui| {
            crate::widgets::row(ui, |ui| {
                for (label, key, pt) in [("W:", "width", w0), ("H:", "height", h0)] {
                    let name = ui.label(egui::RichText::new(tl!(label)).small());
                    let was = pt / k;
                    let mut v = was;
                    // Not `clamp_existing_to_range`: egui would clamp a size outside the range (a 1 px
                    // picture is 0.75 pt) and report it as a change, resizing the picture unasked.
                    let field = egui::DragValue::new(&mut v)
                        .speed(1.0 / k)
                        .range(1.0 / k..=4000.0 / k)
                        .clamp_existing_to_range(false)
                        .measure(unit)
                        .max_decimals(2);
                    let r = ui.add(field.update_while_editing(false)).labelled_by(name.id).on_hover_text(tl!(
                        "Type a size in any unit, such as 5 cm or 300 px. The other side follows, so the picture keeps its proportions."
                    ));
                    // While typing, the field reports each keystroke as a change; only a new
                    // value (Enter, Tab, a click elsewhere, a drag) resizes, and Esc cancels.
                    let cancelled = esc_cancelled(ui, &r, egui::Id::unique(("picture_size_field", key)));
                    if r.changed() && v != was && !cancelled {
                        if r.dragged() && !r.drag_started() {
                            app.session.join_next_undo();
                        }
                        let _ = app.run("picture.size", json!({key: v * k}));
                    }
                }
                rotation_field(ui, app);
            });
            ui.add_space(2.0);
            crate::widgets::row(ui, |ui| {
                let cropping = crate::crop::active(app).is_some();
                small(ui, app, "crop", Some("Crop"), "Crop", crate::widgets::ButtonAction::new("ui.crop", json!({}), cropping));
                for (i, side) in ["L", "T", "R", "B"].iter().enumerate() {
                    ui.label(egui::RichText::new(tl!(side)).small());
                    let mut v = crop0[i] * 100.0;
                    let r = ui.add(egui::DragValue::new(&mut v).speed(0.5).range(0.0..=95.0).suffix("%"));
                    if r.changed() {
                        if r.dragged() && !r.drag_started() {
                            app.session.join_next_undo();
                        }
                        let mut c = crop0;
                        c[i] = (v / 100.0).clamp(0.0, 0.95);
                        // The two sides of an axis keep 5% of the source between them.
                        let other = c[if i < 2 { i + 2 } else { i - 2 }];
                        c[i] = c[i].min((0.95 - other).max(0.0));
                        let _ = app.run("picture.crop", json!({"left": c[0], "top": c[1], "right": c[2], "bottom": c[3]}));
                    }
                }
                if ui.button(egui::RichText::new(tl!("Reset")).small()).on_hover_text(tl!("Reset Crop")).clicked() {
                    let _ = app.run("picture.crop", json!({"left": 0.0, "top": 0.0, "right": 0.0, "bottom": 0.0}));
                }
            });
            ui.add_space(2.0);
            crate::widgets::row(ui, |ui| {
                ui.label(egui::RichText::new(tl!("Alt:")).small());
                // Frame-local buffers lose keystrokes; keep the draft in egui temp memory keyed
                // by picture (same pattern as the comment editor in panes.rs).
                let key = egui::Id::unique(("picture_alt", media.clone(), off));
                let mut alt = ui.data(|d| d.get_temp::<String>(key)).unwrap_or_else(|| alt0.clone());
                let r = ui.add(egui::TextEdit::singleline(&mut alt).desired_width(120.0).hint_text(tl!("Alt text")));
                if r.changed() {
                    ui.data_mut(|d| d.insert_temp(key, alt.clone()));
                }
                if r.lost_focus() {
                    ui.data_mut(|d| d.remove::<String>(key));
                    // The click that took focus may have selected another picture; never commit
                    // one picture's draft onto another.
                    if alt != alt0 && app.selected_picture_media().as_deref() == Some(media.as_str()) {
                        let _ = app.run("picture.altText", json!({"text": alt}));
                    }
                }
            });
        });
    });
}
