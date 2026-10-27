use super::arrange::{align_menu, group_menu, rotate_menu, wrap_menu};
use super::common::{mi, size_menu, stack};
use super::language::mi_check;
use super::*;

// Layout › Margins, Orientation, Size and Columns: the caret section's setting has a check
// mark, as Text Direction's does (#1206).
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
            Some(pt) => crate::i18n::fmt(tl!(l), &[("size", &crate::options::length(pt))]),
            None => l.to_string(),
        };
        // Mirrored margins come from the document as well as from the preset (#1181).
        let on = current == Some(k) || (k == "mirrored" && app.session.doc.settings.mirror_margins);
        mi_check(ui, app, &label, on, "layout.margins", json!({"preset": k}));
    }
    mi(ui, app, "Custom Margins…", "ui.dialog", json!({"name": "pageSetup"}));
}

pub(super) fn orientation_items(ui: &mut Ui, app: &mut WordApp) {
    let land = wordcraft_engine::cmd::page::sect(&app.session).landscape;
    mi_check(ui, app, "Portrait", !land, "layout.orientation", json!({"value": "portrait"}));
    mi_check(ui, app, "Landscape", land, "layout.orientation", json!({"value": "landscape"}));
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
        mi_check(ui, app, l, current == Some(k), "layout.columns", params);
    }
    ui.separator();
    mi(ui, app, "More Columns…", "ui.dialog", json!({"name": "columns"}));
}

pub(super) fn layout(app: &mut WordApp, ui: &mut Ui) {
    group(ui, "Page Setup", Some("layout.pageSetup"), app, |ui, app| {
        // Text Direction works on what the caret is in: table cells, a text box, else the section.
        let dir = wordcraft_engine::cmd::page::text_direction_at(&app.session);
        menu_button(ui, app, "textDirection", Some("Text\nDirection"), "Text Direction", true, "layout.textDirection", |ui, app| {
            use wordcraft_doc::props::TextDirection;
            for (label, d) in
                [("Horizontal", TextDirection::Horizontal), ("Rotate all text 90°", TextDirection::Down), ("Rotate all text 270°", TextDirection::Up)]
            {
                mi_check(ui, app, label, dir == d, "layout.textDirection", json!({"value": d.ooxml()}));
            }
        });
        menu_button(ui, app, "margins", Some("Margins"), "Margins", true, "layout.margins", margins_items);
        menu_button(ui, app, "orientation", Some("Orientation"), "Orientation", true, "layout.orientation", orientation_items);
        menu_button(ui, app, "size", Some("Size"), "Size", true, "layout.size", size_menu);
        menu_button(ui, app, "columns", Some("Columns"), "Columns", true, "layout.columns", columns_items);
        stack(ui, |ui| {
            menu_button(ui, app, "breaks", Some("Breaks"), "Breaks", false, "layout.break", |ui, app| {
                ui.label(egui::RichText::new(tl!("Page Breaks")).strong());
                mi(ui, app, "Page", "layout.break", json!({"kind": "page"}));
                mi(ui, app, "Column", "layout.break", json!({"kind": "column"}));
                mi(ui, app, "Text Wrapping", "layout.break", json!({"kind": "textWrapping"}));
                ui.label(egui::RichText::new(tl!("Section Breaks")).strong());
                mi(ui, app, "Next Page", "layout.break", json!({"kind": "nextPage"}));
                mi(ui, app, "Continuous", "layout.break", json!({"kind": "continuous"}));
                mi(ui, app, "Even Page", "layout.break", json!({"kind": "evenPage"}));
                mi(ui, app, "Odd Page", "layout.break", json!({"kind": "oddPage"}));
            });
            menu_button(ui, app, "lineNumbers", Some("Line Numbers"), "Line Numbers", false, "layout.lineNumbers", |ui, app| {
                for (l, v) in
                    [("None", "none"), ("Continuous", "continuous"), ("Restart Each Page", "restartPage"), ("Restart Each Section", "restartSection")]
                {
                    mi(ui, app, l, "layout.lineNumbers", json!({"value": v}));
                }
                ui.separator();
                mi(ui, app, "Line Numbering Options…", "ui.dialog", json!({"name": "lineNumbers"}));
            });
            let hy = app.session.doc.settings.auto_hyphenation;
            menu_button(ui, app, "hyphenation", Some("Hyphenation"), "Hyphenation", false, "layout.hyphenation", |ui, app| {
                mi(ui, app, if hy { "✓ Automatic" } else { "Automatic" }, "layout.hyphenation", json!({"value": true}));
                mi(ui, app, if hy { "None" } else { "✓ None" }, "layout.hyphenation", json!({"value": false}));
                mi(ui, app, "Manual…", "ui.dialog", json!({"name": "manualHyphenation"}));
                ui.separator();
                mi(ui, app, "Hyphenation Options…", "ui.dialog", json!({"name": "hyphenation"}));
            });
        });
    });
    group(ui, "Paragraph", Some("para.dialog"), app, |ui, app| {
        let rp = app.session.doc.para_at(&app.session.sel.focus).map(|p| app.session.doc.styles.resolve_para(&p.props));
        let Some(rp) = rp else { return };
        // #654: header + two rows of 20 px fields fit the content height above the group title
        // (1.5 pt between rows; 3 ran 2.5 pt into it); labels and fields sit on shared columns,
        // every field the same width.
        const FIELD: egui::Vec2 = egui::Vec2::new(64.0, 20.0);
        ui.spacing_mut().interact_size.y = FIELD.y;
        // The gap before the Spacing column sits inside its cells: `add_space` in a grid moves the
        // next cell in release builds but trips a debug assertion.
        let spaced = |ui: &mut Ui, text: egui::RichText| {
            ui.horizontal(|ui| {
                ui.add_space(8.0);
                ui.label(text);
            });
        };
        egui::Grid::new("layout_para").spacing(vec2(4.0, 1.5)).min_row_height(FIELD.y).show(ui, |ui| {
            ui.label(egui::RichText::new(tl!("Indent")).small().strong());
            ui.label("");
            spaced(ui, egui::RichText::new(tl!("Spacing")).small().strong());
            ui.end_row();
            // Indents in the File › Options unit.
            let u = crate::options::unit();
            let k = u.pt_per_unit();
            let mut l = rp.indent_left / k;
            let mut b = rp.space_before;
            ui.label(egui::RichText::new(tl!("Left:")).small());
            if ui.add_sized(FIELD, egui::DragValue::new(&mut l).speed(3.6 / k).range(-792.0 / k..=1584.0 / k).measure(u).max_decimals(2)).changed() {
                let _ = app.run("para.indents", json!({"left": l * k}));
            }
            spaced(ui, egui::RichText::new(tl!("Before:")).small());
            if ui.add_sized(FIELD, egui::DragValue::new(&mut b).speed(1.0).range(0.0..=1584.0).suffix(format!(" {}", tl!("pt")))).changed() {
                let _ = app.run("para.spacing", json!({"before": b}));
            }
            ui.end_row();
            let mut r = rp.indent_right / k;
            let mut a = rp.space_after;
            ui.label(egui::RichText::new(tl!("Right:")).small());
            if ui.add_sized(FIELD, egui::DragValue::new(&mut r).speed(3.6 / k).range(-792.0 / k..=1584.0 / k).measure(u).max_decimals(2)).changed() {
                let _ = app.run("para.indents", json!({"right": r * k}));
            }
            spaced(ui, egui::RichText::new(tl!("After:")).small());
            if ui.add_sized(FIELD, egui::DragValue::new(&mut a).speed(1.0).range(0.0..=1584.0).suffix(format!(" {}", tl!("pt")))).changed() {
                let _ = app.run("para.spacing", json!({"after": a}));
            }
            ui.end_row();
        });
    });
    group(ui, "Arrange", None, app, |ui, app| {
        big(ui, app, "position", "Position", "arrange.position", json!({}), false);
        wrap_menu(ui, app);
        stack(ui, |ui| {
            small(ui, app, "bringForward", Some("Bring Forward"), "Bring Forward", "arrange.bringForward", json!({}), false);
            small(ui, app, "sendBackward", Some("Send Backward"), "Send Backward", "arrange.sendBackward", json!({}), false);
            small(ui, app, "selectionPane", Some("Selection Pane"), "Selection Pane", "arrange.selectionPane", json!({}), false);
        });
        stack(ui, |ui| {
            align_menu(ui, app, None);
            group_menu(ui, app, None);
            rotate_menu(ui, app, None);
        });
    });
}
