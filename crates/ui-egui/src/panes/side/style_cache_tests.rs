use super::*;

fn app() -> WordApp {
    let mut doc = wordcraft_doc::Document::from_text("First paragraph.\nSecond paragraph.");
    let normal = doc.styles.get("Normal").unwrap().clone();
    for i in 0..256 {
        let mut style = normal.clone();
        style.id = format!("Original{i}");
        style.name = format!("Original style {i:03}");
        style.builtin = false;
        style.hidden = false;
        doc.styles.styles.push(style);
    }
    WordApp::new(wordcraft_engine::Session::new(doc), crate::Services::default())
}

fn paint(ctx: &egui::Context, app: &mut WordApp) {
    ctx.run_ui(
        egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(400.0, 800.0))), ..Default::default() },
        |ui| styles(app, ui),
    )
    .drop_without_applying_deltas();
}

#[test]
fn styles_pane_reuses_unchanged_list_across_repaints() {
    let ctx = egui::Context::default();
    crate::theme::install_fonts(&ctx);
    let mut app = app();
    let before = serde_json::to_value(&app.session.doc).unwrap();
    let revision = app.session.rev();
    STYLE_LIST_BUILDS.with(|n| n.set(0));
    for _ in 0..4 {
        paint(&ctx, &mut app);
    }
    assert_eq!(app.session.rev(), revision);
    assert_eq!(serde_json::to_value(&app.session.doc).unwrap(), before);
    assert_eq!(STYLE_LIST_BUILDS.with(|n| n.get()), 1, "unchanged repaints must reuse the ordered style list");
    app.run("caret.docEnd", json!({})).unwrap();
    paint(&ctx, &mut app);
    assert_eq!(STYLE_LIST_BUILDS.with(|n| n.get()), 1, "caret movement keeps the list");
}

#[test]
fn styles_pane_keeps_stable_order_and_style_flags() {
    use wordcraft_doc::{Document, Style, StyleKind};
    let mut doc = Document::new();
    doc.styles.styles = [
        ("char", "alpha", StyleKind::Character, false, false),
        ("para", "Alpha", StyleKind::Paragraph, false, true),
        ("number", "Beta", StyleKind::Numbering, false, false),
        ("unicode", "Ä original", StyleKind::Paragraph, false, false),
        ("hidden", "A hidden", StyleKind::Paragraph, true, false),
        ("table", "A table", StyleKind::Table, false, false),
    ]
    .into_iter()
    .map(|(id, name, kind, hidden, builtin)| Style { id: id.into(), name: name.into(), kind, hidden, builtin, ..Default::default() })
    .collect();
    let mut app = WordApp::new(wordcraft_engine::Session::new(doc), crate::Services::default());
    let rows = cached_styles(&mut app);
    let actual: Vec<_> = rows.iter().map(|(id, name, chr, builtin)| (id.as_str(), name.as_str(), *chr, *builtin)).collect();
    assert_eq!(
        actual,
        vec![
            ("char", "alpha", true, false),
            ("para", "Alpha", false, true),
            ("number", "Beta", false, false),
            ("unicode", "Ä original", false, false)
        ]
    );
}

#[test]
fn styles_pane_refreshes_translated_order_without_document_edits() {
    struct Restore(crate::i18n::Lang);
    impl Drop for Restore {
        fn drop(&mut self) {
            crate::i18n::set_current(self.0);
        }
    }
    let _restore = Restore(crate::i18n::current());
    crate::i18n::set_current(crate::i18n::Lang::EN);
    let mut app = app();
    app.run("styles.create", json!({"name": "Normal custom"})).unwrap();
    let revision = app.session.rev();
    let english = cached_styles(&mut app);
    assert!(english.iter().any(|(id, name, ..)| id == "Normal" && name == "Normal"));
    let language = crate::i18n::fixture_lang("de", "\tNormal\tZZ translated normal\n\tHeading {n}\tAA heading {n}\n");
    crate::i18n::set_current(language);
    let translated = cached_styles(&mut app);
    assert!(!std::sync::Arc::ptr_eq(&english, &translated));
    let normal = translated.iter().position(|(id, name, ..)| id == "Normal" && name == "ZZ translated normal").unwrap();
    let heading = translated.iter().position(|(id, name, ..)| id == "Heading1" && name.contains("AA heading")).unwrap();
    assert!(heading < normal, "sort translated names, not stored English names");
    assert!(translated.iter().any(|(_, name, ..)| name == "Normal custom"), "custom names remain verbatim");
    assert_eq!(app.session.rev(), revision);
    crate::i18n::set_current(crate::i18n::Lang::EN);
    assert_eq!(*cached_styles(&mut app), *english);
}

#[test]
fn styles_pane_refreshes_after_style_edits_and_history() {
    let ctx = egui::Context::default();
    crate::theme::install_fonts(&ctx);
    let mut app = app();
    paint(&ctx, &mut app);
    let old = cached_styles(&mut app);
    let id = app.run("styles.create", json!({"name": "Aardvark custom"})).unwrap()["id"].as_str().unwrap().to_string();
    paint(&ctx, &mut app);
    assert!(!std::sync::Arc::ptr_eq(&old, &cached_styles(&mut app)));
    assert_eq!(cached_styles(&mut app)[0].1, "Aardvark custom");
    app.run("styles.modify", json!({"style": id, "name": "Zebra renamed"})).unwrap();
    paint(&ctx, &mut app);
    assert!(cached_styles(&mut app).iter().any(|r| r.0 == id && r.1 == "Zebra renamed"));
    app.run("styles.setVisibility", json!({"style": id, "hidden": true})).unwrap();
    paint(&ctx, &mut app);
    assert!(!cached_styles(&mut app).iter().any(|r| r.0 == id));
    app.run("edit.undo", json!({})).unwrap();
    paint(&ctx, &mut app);
    assert!(cached_styles(&mut app).iter().any(|r| r.0 == id));
    app.run("edit.redo", json!({})).unwrap();
    paint(&ctx, &mut app);
    assert!(!cached_styles(&mut app).iter().any(|r| r.0 == id));
    app.run("styles.delete", json!({"style": id})).unwrap();
    app.run("edit.undo", json!({})).unwrap();
    app.run("styles.setVisibility", json!({"style": id, "hidden": false})).unwrap();
    paint(&ctx, &mut app);
    let rows = cached_styles(&mut app);
    assert!(rows.iter().any(|r| r.0 == id));
    app.run("text.insert", json!({"text": "Body edit", "raw": true})).unwrap();
    paint(&ctx, &mut app);
    assert!(!std::sync::Arc::ptr_eq(&rows, &cached_styles(&mut app)), "document revision invalidates the list");
    assert_eq!(*rows, *cached_styles(&mut app), "body edits preserve the style rows");
}

#[test]
fn styles_pane_refreshes_on_equal_revision_document_switch_and_replacement() {
    let mut app = app();
    let first = app.session.document_id();
    let rev = app.session.rev();
    let rows = cached_styles(&mut app);
    let second = app.session.add_document(wordcraft_doc::Document::new());
    assert_eq!(app.session.rev(), rev);
    assert_ne!(first, second);
    assert!(!cached_styles(&mut app).iter().any(|r| r.0 == "Original0"));
    app.session.switch_document(first).unwrap();
    assert_eq!(*cached_styles(&mut app), *rows);
    app.session.set_document(wordcraft_doc::Document::new());
    assert!(!cached_styles(&mut app).iter().any(|r| r.0 == "Original0"));
}

#[test]
#[cfg(not(target_arch = "wasm32"))]
fn styles_pane_cache_follows_window_ownership() {
    let ctx = egui::Context::default();
    crate::theme::install_fonts(&ctx);
    let mut app = app();
    let first = app.windows.shown();
    paint(&ctx, &mut app);
    let weak = std::sync::Arc::downgrade(&app.style_cache.items);
    app.new_window();
    let second = app.windows.shown();
    STYLE_LIST_BUILDS.with(|n| n.set(0));
    for _ in 0..4 {
        for key in [first, second] {
            assert!(app.enter_window(key));
            paint(&ctx, &mut app);
        }
    }
    assert_eq!(STYLE_LIST_BUILDS.with(|n| n.get()), 1, "each window keeps its own list");
    assert!(weak.upgrade().is_some());
    app.close_window(first);
    assert_eq!(app.window_count(), 1);
    assert!(weak.upgrade().is_none(), "closing a window releases its list while egui stays alive");
}

#[test]
#[cfg(not(target_arch = "wasm32"))]
fn styles_pane_cached_rows_keep_actions_live() {
    use egui_kittest::kittest::Queryable;
    let mut app = WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::from_text("Original text")), crate::Services::default());
    let id = app.run("styles.create", json!({"name": "Aardvark custom"})).unwrap()["id"].as_str().unwrap().to_string();
    app.run("para.style", json!({"style": "Normal"})).unwrap();
    let mut h = egui_kittest::Harness::builder()
        .with_size(egui::vec2(400.0, 1200.0))
        .with_context_setup(crate::theme::install_fonts)
        .build_ui_state(|ui, app: &mut WordApp| styles(app, ui), app);
    h.run_steps(4);
    h.get_by_label("Aardvark custom").click();
    h.run_steps(2);
    assert_eq!(h.state().session.doc.para_at(&h.state().session.sel.focus).unwrap().props.style.as_deref(), Some(id.as_str()));
    assert!(h.state().canvas.want_focus);
    h.state_mut().run("select.all", json!({})).unwrap();
    h.state_mut().run("format.bold", json!({"value": true})).unwrap();
    h.run_steps(2);
    h.get_by_label("Update to Match").click();
    h.run_steps(2);
    assert_eq!(h.state().session.doc.styles.get(&id).unwrap().chr.bold, Some(true));
    h.get_by_label("Aardvark custom").click_secondary();
    h.run_steps(2);
    h.get_by_label("Modify…").click();
    h.run_steps(2);
    assert_eq!(h.state().dialog.as_ref().map(crate::dialogs::Dialog::name), Some("modifyStyle"));
    h.state_mut().dialog = None;
    h.get_by_label("Aardvark custom").click_secondary();
    h.run_steps(2);
    h.get_by_label("Delete").click();
    h.run_steps(2);
    assert!(h.state().session.doc.styles.get(&id).is_none());
    assert!(!h.state().style_cache.items.iter().any(|r| r.0 == id));
}
