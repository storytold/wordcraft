//! Dialogs with real tabs: a click on a tab shows its page.

use egui_kittest::kittest::Queryable;
use serde_json::json;
use wordcraft_engine::Session;

use crate::WordApp;
use crate::dialogs::Dialog;

fn harness(name: &str) -> egui_kittest::Harness<'static, WordApp> {
    let mut app = WordApp::new(Session::new(wordcraft_doc::Document::new()), crate::Services::default());
    let _ = app.run("ui.language", json!({"value": "en"}));
    let _ = app.run("document.setText", json!({"text": "Some text"}));
    let _ = app.run("select.all", json!({}));
    let _ = app.run("ui.dialog", json!({"name": name}));
    if app.dialog.is_none() && app.colors.is_none() {
        app.dialog = Dialog::open(name, &mut app);
    }
    let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(1440.0, 900.0)).with_accessibility_check(false).build_ui_state(
        |ui, app: &mut WordApp| {
            let ctx = ui.ctx().clone();
            app.logic(&ctx);
            app.ui(ui);
        },
        app,
    );
    h.ctx.set_fonts(crate::theme::font_definitions(false, false));
    h.ctx.set_missing_glyph_policy(egui::MissingGlyphPolicy::Tofu);
    settle(&mut h);
    h
}

fn settle(h: &mut egui_kittest::Harness<'static, WordApp>) {
    for _ in 0..4 {
        h.step();
    }
}

fn tab_button<'a>(h: &'a egui_kittest::Harness<'static, WordApp>, label: &'a str) -> egui_kittest::Node<'a> {
    h.query_all_by_role_and_label(egui::accesskit::Role::Button, label).last().unwrap_or_else(|| panic!("no {label} button"))
}

/// Click each tab in turn. After every click `page` must report its index.
fn check(name: &str, tabs: &[&str], page: impl Fn(&WordApp) -> usize) {
    let mut h = harness(name);
    assert_eq!(page(h.state()), 0, "{name}: opens on the first tab");
    for (i, label) in tabs.iter().enumerate() {
        tab_button(&h, label).click();
        settle(&mut h);
        assert_eq!(page(h.state()), i, "{name}: the {label} tab shows its page");
    }
}

#[test]
fn colors_tabs_switch_pages() {
    check("colors", &["Standard", "Custom"], |app| app.colors.as_ref().map_or(9, |d| d.tab as usize));
}

#[test]
fn symbol_tabs_switch_pages() {
    check("symbol", &["Symbols", "Special Characters"], |app| match &app.dialog {
        Some(Dialog::Insert(d)) => match &**d {
            crate::dialogs_insert::InsertDialog::Symbol(f) => f.tab as usize,
            _ => 9,
        },
        _ => 9,
    });
}

#[test]
fn borders_tabs_switch_pages() {
    check("borders", &["Borders", "Page Border", "Shading"], |app| match &app.dialog {
        Some(Dialog::Borders { form }) => form.tab as usize,
        _ => 9,
    });
}

#[test]
fn autocorrect_tabs_switch_pages() {
    check("autoCorrect", &["AutoCorrect", "AutoFormat As You Type", "Math AutoCorrect"], |app| match &app.dialog {
        Some(Dialog::Review(d)) => match &**d {
            crate::dialogs_review::ReviewDialog::AutoCorrect(f) => f.tab as usize,
            _ => 9,
        },
        _ => 9,
    });
}

#[test]
fn customize_ribbon_tabs_switch_pages() {
    check("customizeRibbon", &["Ribbon", "Quick Access Toolbar"], |app| match &app.dialog {
        Some(Dialog::CustomizeRibbon { form }) => (form.page == crate::dialogs_ribbon::Page::QuickAccess) as usize,
        _ => 9,
    });
}
