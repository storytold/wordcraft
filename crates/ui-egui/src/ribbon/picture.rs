use craft::behavior::IconId;
use craft::menu::ribbon::{RibbonGroup, RibbonRow};
use craft::ui::buttons::{Arrangement, Button, ToggleButton};
use craft::ui::contract::{Bind, Gap, Props as _, Show as _, ShowContainer as _};
use craft::ui::display::{Divider, Text};
use craft::ui::layout_components::{Row, Stack};
use craft::ui::menu::MenuItem;
use craft::ui::runtime::commands::CommandRef;
use craft::ui::runtime::env::UnitKind;
use craft::ui::style::TypeRole;
use craft::ui::text::{NumberField, TextField};

use super::arrange::{align_menu, group_menu, rotate_menu, rotation_field, wrap_menu};
use super::common::{menu_button, size_field};
use super::shape::alt_text_group;
use super::{WordApp, json};
use crate::colors::Target;
use crate::undo_marks::apply_edits;

/// Fills the Picture Format tab's Adjust, Picture Styles, Accessibility (Alt Text), Arrange and Size groups.
pub(super) fn picture_format(row: &mut RibbonRow<'_>, app: &mut WordApp) {
    row.group(RibbonGroup::new("picture.adjust", tl!("Adjust"), IconId::new("wordcraft.more").into()), |ui| {
        let stacked = |label: &'static str, tip: &'static str| {
            Button::new(tl!(label)).icon(IconId::new("picture")).tooltip(tl!(tip)).arrangement(Arrangement::Stacked)
        };
        menu_button(ui, stacked("Corrections", "Brightness, contrast and sharpness"), tl!("Corrections"), Some("picture.corrections"), |ui, _| {
            for (l, b, c) in
                [("Brighter +20%", 20.0, 0.0), ("Darker −20%", -20.0, 0.0), ("More Contrast +20%", 0.0, 20.0), ("Less Contrast −20%", 0.0, -20.0)]
            {
                MenuItem::command(CommandRef::with_params("picture.corrections", json!({"brightness": b, "contrast": c}))).label(tl!(l)).show(ui);
            }
            Divider::horizontal().show(ui);
            MenuItem::command(CommandRef::with_params("picture.corrections", json!({"sharpen": 40.0}))).label(tl!("Sharpen")).show(ui);
            MenuItem::command(CommandRef::with_params("picture.corrections", json!({"sharpen": -40.0}))).label(tl!("Soften")).show(ui);
        });
        menu_button(ui, stacked("Color", "Saturation and recolor"), tl!("Color"), Some("picture.color"), |ui, _| {
            for (l, v) in [("Full Saturation 100%", 100.0), ("Muted 50%", 50.0), ("Gray 0%", 0.0), ("Vivid 200%", 200.0)] {
                MenuItem::command(CommandRef::with_params("picture.color", json!({"mode": "saturation", "saturation": v}))).label(tl!(l)).show(ui);
            }
            Divider::horizontal().show(ui);
            for (l, m) in [("Grayscale", "grayscale"), ("Sepia", "sepia"), ("Washout", "washout")] {
                MenuItem::command(CommandRef::with_params("picture.color", json!({"mode": m}))).label(tl!(l)).show(ui);
            }
        });
        let icon_color = Button::new(tl!("Icon Color"))
            .icon(IconId::new("icons"))
            .tooltip(tl!("Recolor a one-color picture, such as an icon"))
            .arrangement(Arrangement::Stacked);
        menu_button(ui, icon_color, tl!("Icon Color"), Some("picture.iconColor"), |ui, _| {
            let _ = crate::colors::menu(ui, app, Target::cmd("picture.iconColor"));
        });
        menu_button(ui, stacked("Transparency", "Picture transparency"), tl!("Transparency"), Some("picture.transparency"), |ui, _| {
            for v in [0.0, 15.0, 30.0, 50.0, 65.0, 80.0, 95.0] {
                let label = format!("{v:.0}%");
                MenuItem::command(CommandRef::with_params("picture.transparency", json!({"percent": v})))
                    .label(tl!(label.as_str()).to_string())
                    .show(ui);
            }
            Divider::horizontal().show(ui);
            // The engine has no transparency read, so the exact field starts empty each time the menu opens.
            let mut exact: Option<f64> = None;
            let out = NumberField::new(tl!("Transparency"), Bind::bound(&mut exact)).unit_kind(UnitKind::Percent).range(0.0..=100.0).show(ui);
            apply_edits(app, out.edit.events, |app, v: Option<f64>| {
                if let Some(percent) = v {
                    let _ = app.run("picture.transparency", json!({"percent": percent}));
                }
            });
        });
        Stack::new().gap(Gap::Tight).show(ui, |ui, _| {
            Button::command(CommandRef::new("ui.changePicture"))
                .label(tl!("Change"))
                .icon(IconId::new("picture"))
                .tooltip(tl!("Change Picture"))
                .show(ui);
            Button::command(CommandRef::new("picture.reset")).label(tl!("Reset")).icon(IconId::new("picture")).tooltip(tl!("Reset Picture")).show(ui);
        });
    });
    row.group(RibbonGroup::new("picture.styles", tl!("Picture Styles"), IconId::new("wordcraft.more").into()), |ui| {
        let styles = Button::new(tl!("Styles")).icon(IconId::new("picture")).tooltip(tl!("Frames and effects")).arrangement(Arrangement::Stacked);
        menu_button(ui, styles, tl!("Styles"), Some("picture.style"), |ui, _| {
            for (l, s) in [
                ("Simple Frame", "simpleFrame"),
                ("Thick Frame", "thickFrame"),
                ("Rounded", "rounded"),
                ("Soft Edge", "softEdge"),
                ("Shadow", "shadow"),
            ] {
                MenuItem::command(CommandRef::with_params("picture.style", json!({"style": s}))).label(tl!(l)).show(ui);
            }
        });
        let border = Button::new(tl!("Border")).icon(IconId::new("picture")).tooltip(tl!("Picture border")).arrangement(Arrangement::Stacked);
        menu_button(ui, border, tl!("Border"), Some("picture.border"), |ui, _| {
            for (l, w) in [("Thin", 2), ("Medium", 4), ("Thick", 8)] {
                MenuItem::command(CommandRef::with_params("picture.border", json!({"width": w}))).label(tl!(l)).show(ui);
            }
        });
    });
    alt_text_group(row);
    row.group(RibbonGroup::new("picture.arrange", tl!("Arrange"), IconId::new("wordcraft.more").into()), |ui| {
        Button::command(CommandRef::new("arrange.position")).icon(IconId::new("position")).arrangement(Arrangement::Stacked).show(ui);
        wrap_menu(ui, app);
        Stack::new().gap(Gap::Tight).show(ui, |ui, _| {
            rotate_menu(ui);
            align_menu(ui);
            group_menu(ui, app);
        });
    });
    // The launcher opens the Size dialog: scale, Lock aspect ratio, the original size (#2573).
    row.group(RibbonGroup::new("picture.size", tl!("Size"), IconId::new("wordcraft.more").into()).launcher(CommandRef::new("picture.size")), |ui| {
        let (media, w0, h0, crop0, alt0) = match wordcraft_engine::cmd::objects::selected(&app.session) {
            Some((_, wordcraft_doc::para::InlineObject::Image { media, w, h, alt, crop, .. })) => (media, w, h, crop, alt),
            _ => (String::new(), 0.0, 0.0, [0.0; 4], String::new()),
        };
        Stack::new().gap(Gap::Tight).show(ui, |ui, _| {
            Row::new().show(ui, |ui, _| {
                // `picture.size` locks the aspect ratio, so one side is enough and the other follows.
                size_field(ui, app, "W:", "width", w0, true, 1.0..=4000.0);
                size_field(ui, app, "H:", "height", h0, true, 1.0..=4000.0);
                rotation_field(ui, app);
            });
            Row::new().show(ui, |ui, _| {
                ToggleButton::command(CommandRef::new("ui.crop"))
                    .label(tl!("Crop"))
                    .icon(IconId::new("wordcraft.crop"))
                    .selected(crate::crop::active(app).is_some())
                    .show(ui);
                for (i, side) in ["L", "T", "R", "B"].into_iter().enumerate() {
                    Text::new(tl!(side)).role(TypeRole::Caption).show(ui);
                    let mut v = Some(f64::from(crop0[i]) * 100.0);
                    let out = NumberField::new(tl!(side), Bind::bound(&mut v)).unit_kind(UnitKind::Percent).range(0.0..=95.0).show(ui);
                    apply_edits(app, out.edit.events, |app, v: Option<f64>| {
                        let Some(v) = v else { return };
                        let mut c = crop0;
                        c[i] = (v as f32 / 100.0).clamp(0.0, 0.95);
                        // The two sides of an axis keep 5% of the source between them.
                        let other = c[if i < 2 { i + 2 } else { i - 2 }];
                        c[i] = c[i].min((0.95 - other).max(0.0));
                        let _ = app.run("picture.crop", json!({"left": c[0], "top": c[1], "right": c[2], "bottom": c[3]}));
                    });
                }
                // Reset Crop: every side back to zero.
                Button::command(CommandRef::with_params("picture.crop", json!({"left": 0.0, "top": 0.0, "right": 0.0, "bottom": 0.0})))
                    .label(tl!("Reset"))
                    .tooltip(tl!("Reset Crop"))
                    .show(ui);
            });
            Row::new().show(ui, |ui, _| {
                Text::new(tl!("Alt:")).role(TypeRole::Caption).show(ui);
                let mut alt = alt0.clone();
                let out = TextField::new(tl!("Alt text"), Bind::bound(&mut alt)).id_salt(media.clone()).show(ui);
                apply_edits(app, out.events, |app, text: String| {
                    // The click that took focus may have selected another picture; never commit one picture's
                    // text onto another.
                    if app.selected_picture_media().as_deref() == Some(media.as_str()) {
                        let _ = app.run("picture.altText", json!({"text": text}));
                    }
                });
            });
        });
    });
}
