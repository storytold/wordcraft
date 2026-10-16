//! The ribbon's Breaks menu: its controls carry a name, and each item inserts the break it says.

use serde_json::json;
use wordcraft_engine::Session;
use wordcraft_ui_egui::{Services, WordApp};

/// A WordCraft window on the Layout tab, ready for ribbon clicks.
fn layout_tab() -> egui_kittest::Harness<'static, WordApp> {
    let app = WordApp::new(Session::new(wordcraft_doc::Document::from_text("one\ntwo")), Services::default());
    let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(1440.0, 900.0)).build_ui_state(
        |ui, app: &mut WordApp| {
            let ctx = ui.ctx().clone();
            app.logic(&ctx);
            app.ui(ui);
        },
        app,
    );
    for _ in 0..6 {
        h.step();
    }
    // The test looks for English ribbon labels. An integration test links the library without
    // `cfg!(test)`, so the app follows the system language and a non-English machine labels the
    // ribbon in its own language (#1976): pin the interface language through the app itself.
    h.state_mut().run("ui.language", json!({"value": "en"})).unwrap();
    h.state_mut().run("ui.tab", json!({"tab": "Layout"})).unwrap();
    for _ in 0..3 {
        h.step();
    }
    h
}

/// Open the ribbon's Breaks menu and click the item labelled `item`.
fn click_break(h: &mut egui_kittest::Harness<'static, WordApp>, item: &str) {
    use egui_kittest::kittest::{NodeT, Queryable};
    h.get_by_role_and_label(egui::accesskit::Role::Button, "Breaks").click();
    for _ in 0..3 {
        h.step();
    }
    h.query_all(egui_kittest::kittest::By::new().role(egui::accesskit::Role::Button))
        .find(|n| n.accesskit_node().label().as_deref().map(str::trim) == Some(item))
        .unwrap_or_else(|| panic!("the {item} item is in the Breaks menu"))
        .click();
    for _ in 0..3 {
        h.step();
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

#[test]
fn section_break_from_the_menu_splits_sections() {
    let mut h = layout_tab();
    let before = h.state_mut().session.doc.sections().len();
    click_break(&mut h, "Next Page");
    let after = h.state_mut().session.doc.sections().len();
    assert!(after > before, "the menu section break added a section (was {before}, now {after})");
}
