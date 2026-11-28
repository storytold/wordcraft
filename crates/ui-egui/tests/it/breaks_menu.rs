//! The ribbon's Breaks menu: its controls carry a name, and each item inserts the break it says.

use serde_json::json;
use wordcraft_engine::Session;
use wordcraft_ui_egui::{Services, WordApp};

/// A WordCraft window on the Layout tab, ready for ribbon clicks.
fn layout_tab() -> egui_kittest::Harness<'static, WordApp> {
    let mut app = WordApp::new(Session::new(wordcraft_doc::Document::from_text("one\ntwo")), Services::default());
    // The labels below are English; an integration test otherwise follows the machine's language.
    app.ui.language = "en".into();
    let mut h = crate::harness::builder().with_size(egui::vec2(1440.0, 900.0)).build_ui_state(
        |ui, app: &mut WordApp| {
            let ctx = ui.ctx().clone();
            app.logic(&ctx);
            app.ui(ui);
        },
        app,
    );
    for _ in 0..6 {
        crate::harness::frame(&mut h);
    }
    h.state_mut().run("ui.tab", json!({"tab": "Layout"})).unwrap();
    for _ in 0..3 {
        crate::harness::frame(&mut h);
    }
    h
}

/// Open the ribbon's Breaks menu and click the item labelled `item`.
fn click_break(h: &mut egui_kittest::Harness<'static, WordApp>, item: &str) {
    use egui_kittest::kittest::{NodeT, Queryable};
    h.get_by_role_and_label(egui::accesskit::Role::Button, "Breaks").click();
    for _ in 0..3 {
        crate::harness::frame(h);
    }
    // Menu rows are framework rows, not buttons: match on the name alone.
    h.query_all(egui_kittest::kittest::By::new().label_contains(item))
        .find(|n| n.accesskit_node().label().as_deref().map(str::trim) == Some(item))
        .unwrap_or_else(|| panic!("the {item} item is in the Breaks menu"))
        .click();
    // The frame after the click hands the app the command the menu row fired, which it runs.
    for _ in 0..3 {
        crate::harness::frame(h);
    }
}

#[test]
fn page_break_from_the_menu_adds_a_page() {
    let mut h = layout_tab();
    let before = h.state_mut().session.layout().pages.len();
    click_break(&mut h, "Page");
    let after = h.state_mut().session.layout().pages.len();
    assert!(after > before, "the menu Page break added a page (was {before}, now {after})");
}
