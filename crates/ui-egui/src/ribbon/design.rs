use super::common::menu_button;
use super::{WordApp, json};
use crate::colors::Target;
use craft::behavior::IconId;
use craft::keymap::CommandRef;
use craft::menu::{RibbonGroup, RibbonRow};
use craft::ui::buttons::{Arrangement, Button};
use craft::ui::contract::{Gap, Props as _, Show as _, ShowContainer as _};
use craft::ui::display::Divider;
use craft::ui::layout_components::Stack;
use craft::ui::menu::MenuItem;

pub(super) fn design(row: &mut RibbonRow<'_>, app: &mut WordApp) {
    row.group(RibbonGroup::new("Themes", crate::tl!("Themes"), IconId::new("themes").into()), |ui| {
        menu_button(
            ui,
            Button::new(crate::tl!("Themes")).icon(IconId::new("themes")).tooltip(crate::tl!("Themes")).arrangement(Arrangement::Stacked),
            crate::tl!("Themes"),
            Some("design.theme"),
            |ui, _| {
                for (name, ..) in wordcraft_engine::cmd::design::THEMES {
                    MenuItem::command(CommandRef::with_params("design.theme", json!({"name": name}))).label(crate::tl!(name)).show(ui);
                }
            },
        );
    });
    row.group(RibbonGroup::new("Document Formatting", crate::tl!("Document Formatting"), IconId::new("colors").into()), |ui| {
        super::galleries::style_sets(ui, app);
        Stack::new().gap(Gap::Tight).show(ui, |ui, _| {
            menu_button(
                ui,
                Button::new(crate::tl!("Colors")).icon(IconId::new("colors")).tooltip(crate::tl!("Theme Colors")),
                crate::tl!("Theme Colors"),
                Some("design.themeColors"),
                |ui, _| {
                    for (name, ..) in wordcraft_engine::cmd::design::THEMES {
                        MenuItem::command(CommandRef::with_params("design.themeColors", json!({"name": name}))).label(crate::tl!(name)).show(ui);
                    }
                },
            );
            menu_button(
                ui,
                Button::new(crate::tl!("Fonts")).icon(crate::icons::spec("fonts")).tooltip(crate::tl!("Theme Fonts")),
                crate::tl!("Theme Fonts"),
                Some("design.themeFonts"),
                |ui, _| {
                    for (name, h, b, _) in wordcraft_engine::cmd::design::THEMES {
                        let label = format!("{}: {h} / {b}", crate::tl!(name));
                        MenuItem::command(CommandRef::with_params("design.themeFonts", json!({"heading": h, "body": b}))).label(label).show(ui);
                    }
                },
            );
        });
        Stack::new().gap(Gap::Tight).show(ui, |ui, _| {
            menu_button(
                ui,
                Button::new(crate::tl!("Paragraph Spacing")).icon(IconId::new("paraSpacing")).tooltip(crate::tl!("Paragraph Spacing")),
                crate::tl!("Paragraph Spacing"),
                Some("design.paragraphSpacing"),
                |ui, _| {
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
                        let label = if l == "Open" { crate::i18n::get("spacing", l).unwrap_or(l) } else { crate::tl!(l) };
                        MenuItem::command(CommandRef::with_params("design.paragraphSpacing", json!({"value": v}))).label(label).show(ui);
                    }
                },
            );
            Button::command(CommandRef::with_params("design.effects", json!({})))
                .label(crate::tl!("Effects"))
                .icon(IconId::new("effectsDesign"))
                .show(ui);
            Button::command(CommandRef::with_params("design.setDefault", json!({})))
                .label(crate::tl!("Set as Default"))
                .icon(IconId::new("setDefault"))
                .show(ui);
        });
    });
    row.group(RibbonGroup::new("Page Background", crate::tl!("Page Background"), IconId::new("pageColor").into()), |ui| {
        menu_button(
            ui,
            Button::new(crate::tl!("Watermark"))
                .icon(crate::icons::spec("watermark"))
                .tooltip(crate::tl!("Watermark"))
                .arrangement(Arrangement::Stacked),
            crate::tl!("Watermark"),
            Some("design.watermark"),
            |ui, _| {
                for w in ["DRAFT", "CONFIDENTIAL", "DO NOT COPY", "SAMPLE", "ASAP", "URGENT"] {
                    MenuItem::command(CommandRef::with_params("design.watermark", json!({"text": w}))).label(crate::tl!(w)).show(ui);
                }
                Divider::horizontal().show(ui);
                MenuItem::command(CommandRef::with_params("ui.dialog", json!({"name": "watermark"}))).label(crate::tl!("Custom Watermark…")).show(ui);
                MenuItem::command(CommandRef::with_params("design.watermark", json!({"remove": true})))
                    .label(crate::tl!("Remove Watermark"))
                    .show(ui);
            },
        );
        menu_button(
            ui,
            Button::new(crate::tl!("Page\nColor")).icon(IconId::new("pageColor")).tooltip(crate::tl!("Page Color")).arrangement(Arrangement::Stacked),
            crate::tl!("Page Color"),
            Some("design.pageColor"),
            |ui, _| {
                MenuItem::command(CommandRef::with_params("design.pageColor", json!({"color": null}))).label(crate::tl!("No Color")).show(ui);
                let _ = crate::colors::menu(ui, app, Target::cmd("design.pageColor"));
            },
        );
        Button::command(CommandRef::with_params("design.pageBorders", json!({})))
            .label(crate::tl!("Page\nBorders"))
            .icon(IconId::new("pageBorders"))
            .arrangement(Arrangement::Stacked)
            .show(ui);
    });
}
