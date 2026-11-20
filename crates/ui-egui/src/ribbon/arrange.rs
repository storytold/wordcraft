use craft::behavior::IconId;
use craft::ui::buttons::{Arrangement, Button, SplitButton, SplitMenu};
use craft::ui::contract::{Bind, BodyCx, Props as _, Show as _};
use craft::ui::display::Divider;
use craft::ui::menu::{Menu, MenuItem};
use craft::ui::runtime::commands::CommandRef;
use craft::ui::runtime::env::UnitKind;
use craft::ui::text::NumberField;

use super::common::menu_button;
use super::{Ui, WordApp, json};
use crate::undo_marks::apply_edits;

/// Word's text wrapping styles in menu order: label, model value, `arrange.wrap` parameter.
pub(crate) const WRAP_STYLES: [(&str, wordcraft_doc::para::Wrap, &str); 7] = {
    use wordcraft_doc::para::Wrap;
    [
        ("In Line with Text", Wrap::Inline, "inline"),
        ("Square", Wrap::Square, "square"),
        ("Tight", Wrap::Tight, "tight"),
        ("Through", Wrap::Through, "through"),
        ("Top and Bottom", Wrap::TopAndBottom, "topAndBottom"),
        ("Behind Text", Wrap::BehindText, "behindText"),
        ("In Front of Text", Wrap::InFrontOfText, "inFrontOfText"),
    ]
};

/// The Wrap Text menu for the selected picture, shape or text box: the seven styles (the
/// current one marked), then Move with Text / Fix Position on Page for a floating one. The
/// canvas context menu shows it; the ribbon's Wrap Text button can share it.
pub(crate) fn wrap_text_items(ui: &mut Ui, app: &mut WordApp) {
    use wordcraft_engine::cmd::objects::{moves_with_text, selected};
    let state = selected(&app.session).and_then(|(_, o)| o.frame().map(|(_, _, f)| (f.wrap, moves_with_text(f))));
    let wrap = state.map(|(w, _)| w);
    for (label, w, v) in WRAP_STYLES {
        MenuItem::command(CommandRef::with_params("arrange.wrap", json!({"wrap": v}))).label(tl!(label)).radio_mark(wrap == Some(w)).show(ui);
    }
    Divider::horizontal().show(ui);
    // An inline object is neither: it's part of the text.
    let with_text = state.filter(|(w, _)| *w != wordcraft_doc::para::Wrap::Inline).map(|(_, m)| m);
    MenuItem::command(CommandRef::with_params("arrange.moveWithText", json!({"value": true})))
        .label(tl!("Move with Text"))
        .radio_mark(with_text == Some(true))
        .show(ui);
    MenuItem::command(CommandRef::with_params("arrange.moveWithText", json!({"value": false})))
        .label(tl!("Fix Position on Page"))
        .radio_mark(with_text == Some(false))
        .show(ui);
}

/// Arrange › Wrap Text: how the selected object sits in the text, and (Square, Tight, Through)
/// which of its sides text flows on.
pub(super) fn wrap_menu(ui: &mut Ui, app: &mut WordApp) {
    let (wrap, side) = wordcraft_engine::cmd::objects::selected(&app.session)
        .and_then(|(_, o)| o.frame().map(|(_, _, f)| (f.wrap, f.wrap_text)))
        .unwrap_or_default();
    let face = Button::new(tl!("Wrap\nText")).icon(IconId::new("wrapText")).tooltip(tl!("Wrap Text")).arrangement(Arrangement::Stacked);
    menu_button(ui, face, tl!("Wrap Text"), Some("arrange.wrapText"), |ui, _| {
        use wordcraft_doc::para::WrapText;
        for (l, w, v) in WRAP_STYLES {
            MenuItem::command(CommandRef::with_params("arrange.wrap", json!({"wrap": v}))).label(tl!(l)).radio_mark(wrap == w).show(ui);
        }
        Divider::horizontal().show(ui);
        Menu::submenu_row(ui, tl!("Wrap Text Sides"), |ui, _| {
            for (l, s, v) in [
                ("Both Sides", WrapText::BothSides, "bothSides"),
                ("Left Only", WrapText::Left, "left"),
                ("Right Only", WrapText::Right, "right"),
                ("Largest Only", WrapText::Largest, "largest"),
            ] {
                MenuItem::command(CommandRef::with_params("arrange.wrapText", json!({"side": v}))).label(tl!(l)).radio_mark(side == s).show(ui);
            }
        });
    });
}

/// Arrange › Rotate: turn 90° either way or flip (the angle and mirroring, not the pixels).
pub(super) fn rotate_menu(ui: &mut Ui) {
    let face = Button::new(tl!("Rotate")).icon(IconId::new("rotate"));
    menu_button(ui, face, tl!("Rotate"), Some("arrange.rotate"), |ui, _| {
        for (l, d) in
            [("Rotate Right 90°", "right"), ("Rotate Left 90°", "left"), ("Flip Vertical", "flipVertical"), ("Flip Horizontal", "flipHorizontal")]
        {
            MenuItem::command(CommandRef::with_params("arrange.rotate", json!({"direction": d}))).label(tl!(l)).show(ui);
        }
    });
}

/// Size › Rotation: the selected object's exact angle. A drag of the field is one undo step through the gesture host;
/// an empty field writes nothing.
pub(super) fn rotation_field(ui: &mut Ui, app: &mut WordApp) {
    let mut deg: Option<f64> =
        wordcraft_engine::cmd::objects::selected(&app.session).and_then(|(_, o)| o.frame().map(|(_, _, f)| f64::from(f.spin().deg)));
    let out = NumberField::new(tl!("Rotation"), Bind::bound(&mut deg)).unit_kind(UnitKind::Degrees).range(-360.0..=360.0).commit_decimals(1).show(ui);
    apply_edits(app, out.edit.events, |app, v: Option<f64>| {
        if let Some(degrees) = v {
            let _ = app.run("arrange.rotation", json!({"degrees": degrees}));
        }
    });
}

/// The command the Group menu button follows: Ungroup while a group is selected (Group is off
/// then, as one object has nothing to group), Group otherwise (#4229).
pub(super) fn group_menu_command(app: &WordApp) -> &'static str {
    if app.session.why_disabled("arrange.ungroup").is_none() { "arrange.ungroup" } else { "arrange.group" }
}

/// Arrange › Group: Group (Shift+click objects to select several) and Ungroup.
pub(super) fn group_menu(ui: &mut Ui, app: &mut WordApp) {
    let id = group_menu_command(app);
    SplitButton::<bool>::command(
        CommandRef::new(id),
        SplitMenu::Body(Box::new(|ui: &mut Ui, _: &mut BodyCx<'_>| {
            MenuItem::command(CommandRef::new("arrange.group")).label(tl!("Group")).show(ui);
            MenuItem::command(CommandRef::new("arrange.ungroup")).label(tl!("Ungroup")).show(ui);
        })),
    )
    .label(tl!("Group"))
    // The arrow is a button of its own, so it is named apart from the main part.
    .arrow_label(crate::i18n::fmt(tl!("{name} options"), &[("name", tl!("Group"))]))
    .icon(IconId::new("group"))
    .show(ui);
}

/// Arrange › Align: horizontal position relative to the margin (#3242 — a plain button ran
/// `arrange.align` with no `value`, which always centers; a menu reaches Left and Right too).
pub(super) fn align_menu(ui: &mut Ui) {
    let face = Button::new(tl!("Align")).icon(IconId::new("align"));
    menu_button(ui, face, tl!("Align"), Some("arrange.align"), |ui, _| {
        for (l, v) in [("Align Left", "left"), ("Align Center", "center"), ("Align Right", "right")] {
            MenuItem::command(CommandRef::with_params("arrange.align", json!({"value": v}))).label(tl!(l)).show(ui);
        }
    });
}
