use craft::behavior::{ColumnWidth, IconId};
use craft::menu::ribbon::{RibbonGroup, RibbonRow};
use craft::theme::ControlSize;
use craft::ui::buttons::{Arrangement, Button};
use craft::ui::contract::{Bind, Gap, Props as _, Show as _, ShowContainer as _};
use craft::ui::display::{Divider, Text};
use craft::ui::layout_components::{Grid, Row, Stack};
use craft::ui::menu::MenuItem;
use craft::ui::runtime::commands::CommandRef;
use craft::ui::runtime::env::UnitKind;
use craft::ui::style::TypeRole;
use craft::ui::text::NumberField;

use super::arrange::{align_menu, group_menu, rotate_menu, wrap_menu};
use super::common::{menu_button, size_menu};
use super::{Ui, WordApp, json};
use crate::undo_marks::apply_edits;

/// The columns of each half of the Paragraph group: a label and its field.
const PARAGRAPH_GRID: [ColumnWidth; 2] = [ColumnWidth::Content; 2];

// Layout › Margins, Orientation, Size and Columns: the caret section's setting is marked, as
// Text Direction's is (#1206).
pub(super) fn margins_items(ui: &mut Ui, app: &mut WordApp) {
    let current = wordcraft_engine::cmd::page::margins_preset_at(&app.session);
    // The two uniform presets name their margin in the measurement unit (`2,54 cm`, #838).
    for (l, k, uniform) in [
        ("Normal  {size} all", "normal", Some(72.0)),
        ("Narrow  {size} all", "narrow", Some(36.0)),
        ("Moderate", "moderate", None),
        ("Wide", "wide", None),
        ("Mirrored", "mirrored", None),
        ("Office 2003 Default", "office2003", None),
    ] {
        let label = match uniform {
            Some(pt) => crate::i18n::fmt(tl!(l), &[("size", &app.session.format_length(pt))]),
            None => l.to_string(),
        };
        // Mirrored margins come from the document as well as from the preset (#1181).
        let on = current == Some(k) || (k == "mirrored" && app.session.doc.settings.mirror_margins);
        MenuItem::command(CommandRef::with_params("layout.margins", json!({"preset": k})))
            .label(tl!(label.as_str()).to_string())
            .radio_mark(on)
            .show(ui);
    }
    MenuItem::command(CommandRef::with_params("ui.dialog", json!({"name": "pageSetup"}))).label(tl!("Custom Margins…")).show(ui);
}

pub(super) fn orientation_items(ui: &mut Ui, app: &mut WordApp) {
    let land = wordcraft_engine::cmd::page::sect(&app.session).landscape;
    MenuItem::command(CommandRef::with_params("layout.orientation", json!({"value": "portrait"}))).label(tl!("Portrait")).radio_mark(!land).show(ui);
    MenuItem::command(CommandRef::with_params("layout.orientation", json!({"value": "landscape"}))).label(tl!("Landscape")).radio_mark(land).show(ui);
}

pub(super) fn columns_items(ui: &mut Ui, app: &mut WordApp) {
    let current = wordcraft_engine::cmd::page::columns_preset_at(&app.session);
    for (l, k, params) in [
        ("One", "one", json!({"count": 1})),
        ("Two", "two", json!({"count": 2})),
        ("Three", "three", json!({"count": 3})),
        ("Left", "left", json!({"preset": "left"})),
        ("Right", "right", json!({"preset": "right"})),
    ] {
        MenuItem::command(CommandRef::with_params("layout.columns", params)).label(tl!(l)).radio_mark(current == Some(k)).show(ui);
    }
    Divider::horizontal().show(ui);
    MenuItem::command(CommandRef::with_params("ui.dialog", json!({"name": "columns"}))).label(tl!("More Columns…")).show(ui);
}

/// Fills the Layout ribbon tab (Page Setup, Paragraph, Arrange).
pub(super) fn layout(row: &mut RibbonRow<'_>, app: &mut WordApp) {
    row.group(
        RibbonGroup::new("layout.pageSetup", tl!("Page Setup"), IconId::new("wordcraft.more").into()).launcher(CommandRef::new("layout.pageSetup")),
        |ui| {
            // Text Direction works on what the caret is in: table cells, a text box, else the section.
            let dir = wordcraft_engine::cmd::page::text_direction_at(&app.session);
            let stacked =
                |label: &'static str, icon: &'static str| Button::new(tl!(label)).icon(crate::icons::spec(icon)).arrangement(Arrangement::Stacked);
            menu_button(ui, stacked("Text\nDirection", "textDirection"), tl!("Text Direction"), Some("layout.textDirection"), |ui, _| {
                use wordcraft_doc::props::TextDirection;
                for (label, d) in [
                    ("Horizontal", TextDirection::Horizontal),
                    ("Rotate all text 90°", TextDirection::Down),
                    ("Rotate all text 270°", TextDirection::Up),
                ] {
                    MenuItem::command(CommandRef::with_params("layout.textDirection", json!({"value": d.ooxml()})))
                        .label(tl!(label))
                        .radio_mark(dir == d)
                        .show(ui);
                }
            });
            menu_button(ui, stacked("Margins", "margins"), tl!("Margins"), Some("layout.margins"), |ui, _| margins_items(ui, app));
            menu_button(ui, stacked("Orientation", "orientation"), tl!("Orientation"), Some("layout.orientation"), |ui, _| {
                orientation_items(ui, app)
            });
            menu_button(ui, stacked("Size", "size"), tl!("Size"), Some("layout.size"), |ui, _| size_menu(ui, app));
            menu_button(ui, stacked("Columns", "columns"), tl!("Columns"), Some("layout.columns"), |ui, _| columns_items(ui, app));
            Stack::new().gap(Gap::Tight).show(ui, |ui, _| {
                let inline = |label: &'static str, icon: &'static str| Button::new(tl!(label)).icon(crate::icons::spec(icon));
                menu_button(ui, inline("Breaks", "breaks"), tl!("Breaks"), Some("layout.break"), |ui, _| {
                    MenuItem::heading(tl!("Page Breaks")).show(ui);
                    for (l, k) in [("Page", "page"), ("Column", "column"), ("Text Wrapping", "textWrapping")] {
                        MenuItem::command(CommandRef::with_params("layout.break", json!({"kind": k}))).label(tl!(l)).show(ui);
                    }
                    MenuItem::heading(tl!("Section Breaks")).show(ui);
                    for (l, k) in [("Next Page", "nextPage"), ("Continuous", "continuous"), ("Even Page", "evenPage"), ("Odd Page", "oddPage")] {
                        MenuItem::command(CommandRef::with_params("layout.break", json!({"kind": k}))).label(tl!(l)).show(ui);
                    }
                });
                menu_button(ui, inline("Line Numbers", "lineNumbers"), tl!("Line Numbers"), Some("layout.lineNumbers"), |ui, _| {
                    for (l, v) in [
                        ("None", "none"),
                        ("Continuous", "continuous"),
                        ("Restart Each Page", "restartPage"),
                        ("Restart Each Section", "restartSection"),
                    ] {
                        MenuItem::command(CommandRef::with_params("layout.lineNumbers", json!({"value": v}))).label(tl!(l)).show(ui);
                    }
                    Divider::horizontal().show(ui);
                    MenuItem::command(CommandRef::with_params("ui.dialog", json!({"name": "lineNumbers"})))
                        .label(tl!("Line Numbering Options…"))
                        .show(ui);
                });
                let hy = app.session.doc.settings.auto_hyphenation;
                menu_button(ui, inline("Hyphenation", "hyphenation"), tl!("Hyphenation"), Some("layout.hyphenation"), |ui, _| {
                    MenuItem::command(CommandRef::with_params("layout.hyphenation", json!({"value": true})))
                        .label(tl!("Automatic"))
                        .radio_mark(hy)
                        .show(ui);
                    MenuItem::command(CommandRef::with_params("layout.hyphenation", json!({"value": false})))
                        .label(tl!("None"))
                        .radio_mark(!hy)
                        .show(ui);
                    MenuItem::command(CommandRef::with_params("ui.dialog", json!({"name": "manualHyphenation"}))).label(tl!("Manual…")).show(ui);
                    Divider::horizontal().show(ui);
                    MenuItem::command(CommandRef::with_params("ui.dialog", json!({"name": "hyphenation"})))
                        .label(tl!("Hyphenation Options…"))
                        .show(ui);
                });
            });
            Button::command(CommandRef::new("layout.fitOnePage"))
                .label(tl!("Fit\nOne Page"))
                .icon(IconId::new("fitOnePage"))
                .arrangement(Arrangement::Stacked)
                .show(ui);
        },
    );
    row.group(
        RibbonGroup::new("layout.paragraph", tl!("Paragraph"), IconId::new("alignLeft").into()).launcher(CommandRef::new("para.dialog")),
        |ui| {
            if app.session.doc.para_at(&app.session.sel.focus).is_none() {
                return;
            }
            let rp = wordcraft_engine::cmd::para::selection_para(&app.session);
            let lengths = crate::options::lengths(app.session.units());
            let mut left = Some(f64::from(rp.indent_left));
            let mut right = Some(f64::from(rp.indent_right));
            let mut before = Some(f64::from(rp.space_before));
            let mut after = Some(f64::from(rp.space_after));
            let caption = |label: &'static str| Text::new(tl!(label)).role(TypeRole::Caption);
            // As the old ribbon: two columns of small fields (a header over each label column), labels close to their
            // fields, rows close together, and a wider gap before the Spacing column. Indents show two decimals ("0.00").
            Row::new().gap(Gap::Section).show(ui, |ui, _| {
                Grid::new(&PARAGRAPH_GRID).gap(Gap::Tight).show(ui, |ui, _| {
                    caption("Indent").show(ui);
                    Text::new("").show(ui);
                    caption("Left:").show(ui);
                    let field = NumberField::length(tl!("Left"), Bind::bound(&mut left), lengths).range(-792.0..=1584.0).fixed_decimals(2);
                    let out = field.size(ControlSize::Sm).show(ui);
                    apply_edits(app, out.edit.events, |app, v: Option<f64>| {
                        if let Some(pt) = v {
                            let _ = app.run("para.indents", json!({"left": pt}));
                        }
                    });
                    caption("Right:").show(ui);
                    let field = NumberField::length(tl!("Right"), Bind::bound(&mut right), lengths).range(-792.0..=1584.0).fixed_decimals(2);
                    let out = field.size(ControlSize::Sm).show(ui);
                    apply_edits(app, out.edit.events, |app, v: Option<f64>| {
                        if let Some(pt) = v {
                            let _ = app.run("para.indents", json!({"right": pt}));
                        }
                    });
                });
                Grid::new(&PARAGRAPH_GRID).gap(Gap::Tight).show(ui, |ui, _| {
                    caption("Spacing").show(ui);
                    Text::new("").show(ui);
                    caption("Before:").show(ui);
                    let field = NumberField::new(tl!("Before"), Bind::bound(&mut before)).unit_kind(UnitKind::Points).range(0.0..=1584.0);
                    let out = field.size(ControlSize::Sm).show(ui);
                    apply_edits(app, out.edit.events, |app, v: Option<f64>| {
                        if let Some(pt) = v {
                            let _ = app.run("para.spacing", json!({"before": pt}));
                        }
                    });
                    caption("After:").show(ui);
                    let field = NumberField::new(tl!("After"), Bind::bound(&mut after)).unit_kind(UnitKind::Points).range(0.0..=1584.0);
                    let out = field.size(ControlSize::Sm).show(ui);
                    apply_edits(app, out.edit.events, |app, v: Option<f64>| {
                        if let Some(pt) = v {
                            let _ = app.run("para.spacing", json!({"after": pt}));
                        }
                    });
                });
            });
        },
    );
    row.group(RibbonGroup::new("layout.arrange", tl!("Arrange"), IconId::new("wordcraft.more").into()), |ui| {
        Button::command(CommandRef::new("arrange.position")).icon(IconId::new("position")).arrangement(Arrangement::Stacked).show(ui);
        wrap_menu(ui, app);
        Stack::new().gap(Gap::Tight).show(ui, |ui, _| {
            Button::command(CommandRef::new("arrange.bringForward")).icon(IconId::new("bringForward")).show(ui);
            Button::command(CommandRef::new("arrange.sendBackward")).icon(IconId::new("sendBackward")).show(ui);
            Button::command(CommandRef::new("arrange.selectionPane")).icon(IconId::new("selectionPane")).show(ui);
        });
        // Align, Group and Rotate show their icons alone, as the old ribbon showed them.
        super::common::icon_only(ui, |ui| {
            Stack::new().gap(Gap::Tight).show(ui, |ui, _| {
                align_menu(ui);
                group_menu(ui, app);
                rotate_menu(ui);
            });
        });
    });
}
