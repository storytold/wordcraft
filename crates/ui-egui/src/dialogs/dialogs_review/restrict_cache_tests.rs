use super::*;

fn app() -> WordApp {
    let mut doc = wordcraft_doc::Document::from_text("Original paragraph.");
    for i in 0..256 {
        doc.styles.styles.push(wordcraft_doc::Style { id: format!("custom-{i}"), name: format!("Custom {i}"), ..Default::default() });
    }
    let mut app = WordApp::new(wordcraft_engine::Session::new(doc), crate::Services::default());
    app.restrict.formatting = true;
    app.restrict.show_styles = true;
    app
}

fn paint(ctx: &egui::Context, app: &mut WordApp) {
    ctx.run_ui(
        egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(400.0, 800.0))), ..Default::default() },
        |ui| restrict_pane(app, ui),
    )
    .drop_without_applying_deltas();
}

#[test]
fn restrict_pane_reuses_unchanged_style_list() {
    let ctx = egui::Context::default();
    crate::theme::install_fonts(&ctx);
    let mut app = app();
    let before = serde_json::to_value(&app.session.doc).unwrap();
    let revision = app.session.rev();
    RESTRICT_LIST_BUILDS.with(|n| n.set(0));
    for _ in 0..4 {
        paint(&ctx, &mut app);
    }
    assert_eq!(serde_json::to_value(&app.session.doc).unwrap(), before);
    assert_eq!(app.session.rev(), revision);
    assert_eq!(RESTRICT_LIST_BUILDS.with(|n| n.get()), 1, "unchanged repaints must reuse the ordered restriction styles");
    app.run("caret.docEnd", json!({})).unwrap();
    paint(&ctx, &mut app);
    assert_eq!(RESTRICT_LIST_BUILDS.with(|n| n.get()), 1);
}

#[test]
fn restrict_pane_keeps_stable_order_filtering_and_fallback_names() {
    use wordcraft_doc::{Document, Style, StyleKind};
    let mut doc = Document::new();
    doc.styles.styles = [
        ("chr", "alpha", StyleKind::Character, false),
        ("para", "Alpha", StyleKind::Paragraph, false),
        ("Blank name", "", StyleKind::Paragraph, false),
        ("unicode", "Ä original", StyleKind::Paragraph, false),
        ("hidden", "A hidden", StyleKind::Paragraph, true),
        ("table", "A table", StyleKind::Table, false),
        ("number", "A number", StyleKind::Numbering, false),
    ]
    .into_iter()
    .map(|(id, name, kind, hidden)| Style { id: id.into(), name: name.into(), kind, hidden, ..Default::default() })
    .collect();
    let app = WordApp::new(wordcraft_engine::Session::new(doc), crate::Services::default());
    let mut cache = RestrictStyleCache::default();
    assert_eq!(
        *listed_styles(&app, &mut cache),
        vec![
            ("chr".into(), "alpha".into()),
            ("para".into(), "Alpha".into()),
            ("Blank name".into(), "Blank name".into()),
            ("unicode".into(), "Ä original".into())
        ]
    );
}

#[test]
fn restrict_pane_refreshes_after_style_edits_history_and_replacement() {
    let ctx = egui::Context::default();
    crate::theme::install_fonts(&ctx);
    let mut app = app();
    paint(&ctx, &mut app);
    let old = app.restrict.styles.rows.clone();
    let id = app.run("styles.create", json!({"name":"Aardvark new"})).unwrap()["id"].as_str().unwrap().to_string();
    paint(&ctx, &mut app);
    assert!(!std::sync::Arc::ptr_eq(&old, &app.restrict.styles.rows));
    assert!(app.restrict.styles.rows.iter().any(|r| r.0 == id));
    app.run("styles.modify", json!({"style":id,"name":"Amber renamed"})).unwrap();
    paint(&ctx, &mut app);
    assert!(app.restrict.styles.rows.iter().any(|r| r.0 == id && r.1 == "Amber renamed"));
    app.run("styles.setVisibility", json!({"style":id,"hidden":true})).unwrap();
    paint(&ctx, &mut app);
    assert!(!app.restrict.styles.rows.iter().any(|r| r.0 == id));
    app.run("edit.undo", json!({})).unwrap();
    paint(&ctx, &mut app);
    assert!(app.restrict.styles.rows.iter().any(|r| r.0 == id));
    app.run("edit.redo", json!({})).unwrap();
    paint(&ctx, &mut app);
    assert!(!app.restrict.styles.rows.iter().any(|r| r.0 == id));
    let mut replacement = wordcraft_engine::Session::new(wordcraft_doc::Document::new());
    while replacement.rev() < app.session.rev() {
        replacement.run("text.insert", &json!({"text":"x","raw":true})).unwrap();
    }
    assert_eq!(replacement.rev(), app.session.rev());
    app.session = replacement;
    paint(&ctx, &mut app);
    assert!(!app.restrict.styles.rows.iter().any(|r| r.0.starts_with("custom-")));
}

#[test]
fn restrict_pane_initial_allowed_styles_keep_first_id_lock_semantics() {
    let ctx = egui::Context::default();
    crate::theme::install_fonts(&ctx);
    let mut app = app();
    app.session.doc.styles.styles = [("same", "Alpha", true), ("same", "Beta", false), ("free", "Free", false)]
        .into_iter()
        .map(|(id, name, locked)| wordcraft_doc::Style { id: id.into(), name: name.into(), locked, ..Default::default() })
        .collect();
    paint(&ctx, &mut app);
    assert_eq!(app.restrict.allowed.as_ref().unwrap(), &vec!["free".to_string()]);
}

#[test]
#[cfg(not(target_arch = "wasm32"))]
fn restrict_pane_checkboxes_all_none_and_enforcement_stay_live() {
    use egui_kittest::kittest::Queryable;
    let mut app = WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::from_text("Original paragraph.")), crate::Services::default());
    let id = app.run("styles.create", json!({"name":"Aardvark custom"})).unwrap()["id"].as_str().unwrap().to_string();
    app.restrict.formatting = true;
    app.restrict.show_styles = true;
    let mut h = egui_kittest::Harness::builder()
        .with_size(egui::vec2(400.0, 1200.0))
        .with_context_setup(crate::theme::install_fonts)
        .build_ui_state(|ui, app: &mut WordApp| restrict_pane(app, ui), app);
    h.run_steps(4);
    h.get_by_label("None").click();
    h.run_steps(2);
    assert!(h.state().restrict.allowed.as_ref().unwrap().is_empty());
    h.get_by_label("All").click();
    h.run_steps(2);
    assert!(h.state().restrict.allowed.as_ref().unwrap().contains(&id));
    h.get_by_label("None").click();
    h.run_steps(2);
    h.get_by_label("Aardvark custom").click();
    h.run_steps(2);
    assert_eq!(h.state().restrict.allowed.as_ref().unwrap(), &vec![id.clone()]);
    h.get_by_label("Start Enforcing Protection").click();
    h.run_steps(2);
    h.get_by_label("Cancel").click();
    h.run_steps(2);
    assert!(h.state().session.doc.settings.protection.is_none());
    h.get_by_label("Start Enforcing Protection").click();
    h.run_steps(2);
    h.get_by_label("OK").click();
    h.run_steps(2);
    assert!(h.state().session.doc.settings.protect_formatting);
    assert!(!h.state().session.doc.styles.get(&id).unwrap().locked);
    assert!(h.state().session.doc.styles.get("Normal").unwrap().locked);
    assert!(h.state_mut().run("para.style", json!({"style":"Normal"})).is_err());
    h.state_mut().run("para.style", json!({"style":id})).unwrap();
    assert!(h.state_mut().run("format.bold", json!({"value":true})).is_err());
}

#[test]
#[cfg(not(target_arch = "wasm32"))]
fn restrict_pane_translates_labels_live_without_reordering_source_names() {
    use egui_kittest::kittest::Queryable;
    struct Restore(crate::i18n::Lang);
    impl Drop for Restore {
        fn drop(&mut self) {
            crate::i18n::set_current(self.0);
        }
    }
    let _restore = Restore(crate::i18n::current());
    crate::i18n::set_current(crate::i18n::Lang::EN);
    let mut app = WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::new()), crate::Services::default());
    app.run("styles.create", json!({"name":"Aardvark custom"})).unwrap();
    app.restrict.formatting = true;
    app.restrict.show_styles = true;
    let mut h = egui_kittest::Harness::builder()
        .with_size(egui::vec2(400.0, 1200.0))
        .with_context_setup(crate::theme::install_fonts)
        .build_ui_state(|ui, app: &mut WordApp| restrict_pane(app, ui), app);
    h.run_steps(4);
    h.get_by_label("Aardvark custom");
    let rows = h.state().restrict.styles.rows.clone();
    let revision = h.state().session.rev();
    crate::i18n::set_current(crate::i18n::fixture_lang("de", "\tAardvark custom\tAAA translated\n"));
    h.run_steps(2);
    h.get_by_label("AAA translated");
    assert!(std::sync::Arc::ptr_eq(&rows, &h.state().restrict.styles.rows));
    assert_eq!(h.state().session.rev(), revision);
}

#[test]
#[cfg(not(target_arch = "wasm32"))]
fn restrict_pane_cache_follows_window_ownership_and_close() {
    let ctx = egui::Context::default();
    crate::theme::install_fonts(&ctx);
    let mut app = app();
    let first = app.windows.shown();
    paint(&ctx, &mut app);
    let weak = std::sync::Arc::downgrade(&app.restrict.styles.rows);
    app.new_window();
    let second = app.windows.shown();
    RESTRICT_LIST_BUILDS.with(|n| n.set(0));
    for _ in 0..4 {
        for key in [first, second] {
            assert!(app.enter_window(key));
            paint(&ctx, &mut app);
        }
    }
    assert_eq!(RESTRICT_LIST_BUILDS.with(|n| n.get()), 1);
    assert!(weak.upgrade().is_some());
    app.close_window(first);
    assert!(weak.upgrade().is_none());
}
