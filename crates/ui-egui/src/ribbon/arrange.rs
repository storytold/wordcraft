use super::common::mi;
use super::language::mi_check;
use super::{Ui, WordApp, json, menu_button};

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
/// current one checked), then Move with Text / Fix Position on Page for a floating one. The
/// canvas context menu shows it; the ribbon's Wrap Text button can share it.
pub(crate) fn wrap_text_items(ui: &mut Ui, app: &mut WordApp) {
    use wordcraft_engine::cmd::objects::{moves_with_text, selected};
    let state = selected(&app.session).and_then(|(_, o)| o.frame().map(|(_, _, f)| (f.wrap, moves_with_text(f))));
    let wrap = state.map(|(w, _)| w);
    for (label, w, v) in WRAP_STYLES {
        mi_check(ui, app, label, wrap == Some(w), "arrange.wrap", json!({"wrap": v}));
    }
    ui.separator();
    // An inline object is neither: it's part of the text.
    let with_text = state.filter(|(w, _)| *w != wordcraft_doc::para::Wrap::Inline).map(|(_, m)| m);
    mi_check(ui, app, "Move with Text", with_text == Some(true), "arrange.moveWithText", json!({"value": true}));
    mi_check(ui, app, "Fix Position on Page", with_text == Some(false), "arrange.moveWithText", json!({"value": false}));
}

/// Arrange › Rotate: turn 90° either way or flip (the angle and mirroring, not the pixels).
/// Arrange › Wrap Text: how the selected object sits in the text, and (Square, Tight, Through)
/// which of its sides text flows on.
pub(super) fn wrap_menu(ui: &mut Ui, app: &mut WordApp) {
    let (wrap, side) = wordcraft_engine::cmd::objects::selected(&app.session)
        .and_then(|(_, o)| o.frame().map(|(_, _, f)| (f.wrap, f.wrap_text)))
        .unwrap_or_default();
    menu_button(ui, app, crate::widgets::MenuButtonOptions::new("wrapText", Some("Wrap\nText"), "Wrap Text", true, "arrange.wrapText"), |ui, app| {
        use wordcraft_doc::para::{Wrap, WrapText};
        for (l, w, v) in [
            ("In Line with Text", Wrap::Inline, "inline"),
            ("Square", Wrap::Square, "square"),
            ("Tight", Wrap::Tight, "tight"),
            ("Through", Wrap::Through, "through"),
            ("Top and Bottom", Wrap::TopAndBottom, "topAndBottom"),
            ("Behind Text", Wrap::BehindText, "behindText"),
            ("In Front of Text", Wrap::InFrontOfText, "inFrontOfText"),
        ] {
            mi_check(ui, app, l, wrap == w, "arrange.wrap", json!({"wrap": v}));
        }
        ui.separator();
        ui.menu_button(tl!("Wrap Text Sides"), |ui| {
            for (l, s, v) in [
                ("Both Sides", WrapText::BothSides, "bothSides"),
                ("Left Only", WrapText::Left, "left"),
                ("Right Only", WrapText::Right, "right"),
                ("Largest Only", WrapText::Largest, "largest"),
            ] {
                mi_check(ui, app, l, side == s, "arrange.wrapText", json!({"side": v}));
            }
        });
    });
}

pub(super) fn rotate_menu(ui: &mut Ui, app: &mut WordApp, label: Option<&str>) {
    menu_button(ui, app, crate::widgets::MenuButtonOptions::new("rotate", label, "Rotate", false, "arrange.rotate"), |ui, app| {
        mi(ui, app, "Rotate Right 90°", "arrange.rotate", json!({"direction": "right"}));
        mi(ui, app, "Rotate Left 90°", "arrange.rotate", json!({"direction": "left"}));
        mi(ui, app, "Flip Vertical", "arrange.rotate", json!({"direction": "flipVertical"}));
        mi(ui, app, "Flip Horizontal", "arrange.rotate", json!({"direction": "flipHorizontal"}));
    });
}

/// Size › Rotation: the selected object's exact angle (a drag of the field is one undo step).
pub(super) fn rotation_field(ui: &mut Ui, app: &mut WordApp) {
    let deg0 = wordcraft_engine::cmd::objects::selected(&app.session).and_then(|(_, o)| o.frame().map(|(_, _, f)| f.spin().deg)).unwrap_or(0.0);
    ui.label(egui::RichText::new(tl!("Rotation:")).small());
    let mut deg = deg0;
    let r = ui.add(egui::DragValue::new(&mut deg).speed(1.0).range(-360.0..=360.0).max_decimals(1).suffix("°"));
    if r.changed() {
        if r.dragged() && !r.drag_started() {
            app.session.join_next_undo();
        }
        let _ = app.run("arrange.rotation", json!({"degrees": deg}));
    }
}

/// The command the Group menu button follows: Ungroup while a group is selected (Group is off
/// then, as one object has nothing to group), Group otherwise (#4229).
pub(super) fn group_menu_command(app: &WordApp) -> &'static str {
    if crate::widgets::enabled(app, "arrange.ungroup") { "arrange.ungroup" } else { "arrange.group" }
}

/// Arrange › Group: Group (Shift+click objects to select several) and Ungroup.
pub(super) fn group_menu(ui: &mut Ui, app: &mut WordApp, label: Option<&str>) {
    let id = group_menu_command(app);
    menu_button(ui, app, crate::widgets::MenuButtonOptions::new("group", label, "Group", false, id), |ui, app| {
        mi(ui, app, "Group", "arrange.group", json!({}));
        mi(ui, app, "Ungroup", "arrange.ungroup", json!({}));
    });
}

/// Arrange › Align: horizontal position relative to the margin (#3242 — a plain button ran
/// `arrange.align` with no `value`, which always centers; a menu reaches Left and Right too).
pub(super) fn align_menu(ui: &mut Ui, app: &mut WordApp, label: Option<&str>) {
    menu_button(ui, app, crate::widgets::MenuButtonOptions::new("align", label, "Align", false, "arrange.align"), |ui, app| {
        mi(ui, app, "Align Left", "arrange.align", json!({"value": "left"}));
        mi(ui, app, "Align Center", "arrange.align", json!({"value": "center"}));
        mi(ui, app, "Align Right", "arrange.align", json!({"value": "right"}));
    });
}
