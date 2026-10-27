use super::common::stack;
use super::*;

/// Draws the View tab's Views, Immersive, Page Movement, Show, Zoom, Dark Mode, Window and Macros
/// and Templates groups.
pub(super) fn view(app: &mut WordApp, ui: &mut Ui) {
    let v = app.session.view.clone();
    group(ui, "Views", None, app, |ui, app| {
        big(ui, app, "readMode", "Read\nMode", "view.readMode", json!({}), false);
        big(ui, app, "printLayout", "Print\nLayout", "view.printLayout", json!({}), false);
        big(ui, app, "webLayout", "Web\nLayout", "view.webLayout", json!({}), false);
        stack(ui, |ui| {
            small(ui, app, "outline", Some("Outline"), "Outline", "view.outline", json!({}), false);
            small(ui, app, "draft", Some("Draft"), "Draft", "view.draft", json!({}), false);
        });
    });
    group(ui, "Immersive", None, app, |ui, app| {
        big(ui, app, "focus", "Focus", "view.focus", json!({}), false);
        big(ui, app, "immersive", "Immersive\nReader", "view.immersive", json!({}), false);
    });
    group(ui, "Page Movement", None, app, |ui, app| {
        big(ui, app, "vertical", "Vertical", "view.vertical", json!({}), false);
        big(ui, app, "sideToSide", "Side\nto Side", "view.sideToSide", json!({}), false);
    });
    group(ui, "Show", None, app, |ui, app| {
        // Three rows fill the group's height; a fourth check box would run into its label.
        stack(ui, |ui| {
            let mut r = v.ruler;
            if ui.checkbox(&mut r, tl!("Ruler")).changed() {
                let _ = app.run("view.ruler", json!({"value": r}));
            }
            let mut g = v.gridlines;
            if ui.checkbox(&mut g, tl!("Gridlines")).changed() {
                let _ = app.run("view.gridlines", json!({"value": g}));
            }
            let mut n = v.nav_pane;
            if ui.checkbox(&mut n, tl!("Navigation Pane")).changed() {
                let _ = app.run("view.navigationPane", json!({"value": n}));
            }
        });
        stack(ui, |ui| {
            let mut w = v.hide_white_space;
            if ui.checkbox(&mut w, tl!("Hide White Space")).changed() {
                let _ = app.run("view.hideWhiteSpace", json!({"value": w}));
            }
        });
    });
    group(ui, "Zoom", None, app, |ui, app| {
        big(ui, app, "zoom", "Zoom", "ui.dialog", json!({"name": "zoom"}), false);
        big(ui, app, "zoom100", "100%", "view.zoom100", json!({}), false);
        // Step the zoom up and down by 10% (issue #67), from whatever the page shows now.
        stack(ui, |ui| {
            small(ui, app, "zoomIn", Some("Zoom In"), "Zoom In", "view.zoomIn", json!({}), false);
            small(ui, app, "zoomOut", Some("Zoom Out"), "Zoom Out", "view.zoomOut", json!({}), false);
        });
        stack(ui, |ui| {
            small(ui, app, "onePage", Some("One Page"), "One Page", "view.onePage", json!({}), v.fit == "onePage");
            small(ui, app, "multiplePages", Some("Multiple Pages"), "Multiple Pages", "view.multiplePages", json!({}), v.multi_page);
            small(ui, app, "pageWidth", Some("Page Width"), "Page Width", "view.pageWidth", json!({}), v.fit == "pageWidth");
        });
    });
    group(ui, "Dark Mode", None, app, |ui, app| {
        big(ui, app, "darkMode", "Switch\nModes", "view.darkMode", json!({}), false);
        menu_button(ui, app, "interfaceTheme", Some("Interface\nTheme"), "Interface Theme", true, "", |ui, app| {
            for a in crate::theme::Appearance::ALL {
                if ui.add(egui::Button::selectable(app.ui.theme == a, tl!(a.label())).min_size(vec2(200.0, 0.0))).clicked() {
                    let _ = app.run("view.interfaceTheme", json!({"value": a.code()}));
                    ui.close();
                }
            }
        });
    });
    group(ui, "Window", None, app, |ui, app| {
        big(ui, app, "newWindow", "New\nWindow", "view.newWindow", json!({}), false);
        big(ui, app, "arrangeAll", "Arrange\nAll", "view.arrangeAll", json!({}), false);
        big(ui, app, "split", "Split", "view.split", json!({}), false);
        menu_button(
            ui,
            app,
            "switchWindows",
            Some("Switch\nWindows"),
            "Switch Windows",
            true,
            "view.switchWindows",
            crate::documents::switch_windows_menu,
        );
        // The Full Screen button carries its name for assistive technology (and for the
        // regression test that clicks it): big buttons are otherwise drawn unlabeled. Its
        // enabled state is the command's own, as every other ribbon control reports: the
        // browser has no full screen to ask for, and says so (#662).
        let on = crate::widgets::enabled(app, "view.fullScreen");
        let resp = big_toggle(ui, app, "fullScreen", "Full\nScreen", "view.fullScreen", v.full_screen);
        resp.widget_info(|| egui::WidgetInfo::selected(egui::Role::Button, on, v.full_screen, "Full Screen"));
    });
    group(ui, "Macros", None, app, |ui, app| {
        big(ui, app, "macros", "Macros", "tools.macros", json!({}), false);
    });
    group(ui, "Templates", None, app, |ui, app| {
        big(ui, app, "documentTemplate", "Templates", "tools.templates", json!({}), false);
    });
}

/// The Outlining tab (Outline view): outline levels, moving, expanding and collapsing, what is
/// shown, and closing the view.
pub(super) fn outlining(app: &mut WordApp, ui: &mut Ui) {
    use wordcraft_engine::layout::outline::{BODY, para_level};
    let o = app.session.view.outline.clone();
    let level = app.session.doc.para_at(&app.session.sel.focus).map(|p| para_level(&app.session.doc, p)).unwrap_or(BODY);
    let name = |l: u8| if l >= BODY { tl!("Body Text").to_string() } else { crate::i18n::fmt(tl!("Level {n}"), &[("n", &l.to_string())]) };
    group(ui, "Outline Tools", None, app, |ui, app| {
        stack(ui, |ui| {
            crate::widgets::row(ui, |ui| {
                small(ui, app, "outlineTop", None, "Promote to Heading 1", "outline.promoteToHeading1", json!({}), false);
                small(ui, app, "outlinePromote", None, "Promote", "outline.promote", json!({}), false);
                let levels: Vec<String> = (1..=BODY).map(name).collect();
                if let Some(v) = crate::widgets::combo_hint(ui, "outlineLevel", 100.0, &name(level), &levels, None, tl!("Outline Level"))
                    && let Some(i) = levels.iter().position(|x| *x == v)
                {
                    let l = if i + 1 >= usize::from(BODY) { json!("body") } else { json!(i + 1) };
                    let _ = app.run("outline.level", json!({ "level": l }));
                }
                small(ui, app, "outlineDemote", None, "Demote", "outline.demote", json!({}), false);
                small(ui, app, "outlineBody", None, "Demote to Body Text", "outline.demoteToBody", json!({}), false);
            });
            ui.add_space(3.0);
            crate::widgets::row(ui, |ui| {
                small(ui, app, "outlineUp", None, "Move Up", "outline.moveUp", json!({}), false);
                small(ui, app, "outlineDown", None, "Move Down", "outline.moveDown", json!({}), false);
                small(ui, app, "outlineExpand", None, "Expand", "outline.expand", json!({}), false);
                small(ui, app, "outlineCollapse", None, "Collapse", "outline.collapse", json!({}), false);
                ui.add_space(6.0);
                ui.label(egui::RichText::new(tl!("Show Level")).size(11.5));
                let mut shown: Vec<String> = vec![tl!("All Levels").to_string()];
                shown.extend((1..BODY).map(name));
                let current = shown.get(usize::from(o.show_level)).cloned().unwrap_or_default();
                if let Some(v) = crate::widgets::combo_hint(ui, "outlineShowLevel", 90.0, &current, &shown, None, tl!("Show Level"))
                    && let Some(i) = shown.iter().position(|x| *x == v)
                {
                    let l = if i == 0 { json!("all") } else { json!(i) };
                    let _ = app.run("outline.showLevel", json!({ "level": l }));
                }
            });
            ui.add_space(3.0);
            crate::widgets::row(ui, |ui| {
                let mut f = !o.hide_formatting;
                if ui.checkbox(&mut f, tl!("Show Text Formatting")).changed() {
                    let _ = app.run("outline.showFormatting", json!({ "value": f }));
                }
                let mut first = o.first_line_only;
                if ui.checkbox(&mut first, tl!("Show First Line Only")).changed() {
                    let _ = app.run("outline.firstLineOnly", json!({ "value": first }));
                }
            });
        });
    });
    group(ui, "Close", None, app, |ui, app| {
        big(ui, app, "closeOutline", "Close Outline\nView", "outline.close", json!({}), false);
    });
}
