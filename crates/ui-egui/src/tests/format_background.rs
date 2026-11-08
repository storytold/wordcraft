//! Draw › Edit › Format Background drives the page's fill through its pane, one undo step an Apply.

use egui_kittest::kittest::Queryable;
use serde_json::json;

use crate::WordApp;

/// An app whose document is one paragraph, with the Format Background pane open.
fn app_with_pane() -> egui_kittest::Harness<'static, WordApp> {
    let mut a = WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::from_text("Background")), Default::default());
    a.run("view.formatBackgroundPane", json!({"value": true})).unwrap();
    let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(1440.0, 900.0)).build_ui_state(
        |ui, app: &mut WordApp| {
            let ctx = ui.ctx().clone();
            app.logic(&ctx);
            app.ui(ui);
        },
        a,
    );
    h.run_steps(3);
    h
}

#[test]
fn the_pane_applies_a_gradient_then_a_pattern_as_one_step_each() {
    let mut h = app_with_pane();
    assert!(h.state().session.view.format_background_pane, "the ribbon's command opened the pane");
    h.get_by_label("Gradient").click();
    h.run_steps(2);
    // The preview shows the page; Apply writes the fill.
    let depth = h.state().session.undo_depth();
    h.get_by_label("Apply").click();
    h.run_steps(3);
    let g = h.state().session.doc.settings.page_gradient.clone().expect("gradient applied");
    assert!(g.stops.len() >= 2);
    assert_eq!(h.state().session.undo_depth(), depth + 1, "one undo step");

    h.get_by_label("Pattern").click();
    h.run_steps(2);
    h.get_by_label("10%").click();
    h.run_steps(2);
    let depth = h.state().session.undo_depth();
    h.get_by_label("Apply").click();
    h.run_steps(3);
    let p = h.state().session.doc.settings.page_pattern.clone().expect("pattern applied");
    assert_eq!(p.preset, "pct10");
    assert_eq!(h.state().session.undo_depth(), depth + 1);

    // Undo takes the pattern off and leaves the gradient, the pane reading the document's fill.
    assert!(h.state_mut().session.undo());
    h.run_steps(2);
    assert!(h.state().session.doc.settings.page_pattern.is_none());
    assert!(h.state().session.doc.settings.page_gradient.is_some());
}

#[test]
fn close_hides_the_pane_and_reset_reads_the_document_again() {
    let mut h = app_with_pane();
    h.get_by_label("Solid").click();
    h.run_steps(2);
    // Reset before Apply: nothing in the document changed, so no undo step was added.
    let depth = h.state().session.undo_depth();
    h.get_by_label("Reset").click();
    h.run_steps(2);
    assert_eq!(h.state().session.undo_depth(), depth);
    // The pane's own Close and the host's close control share the label; either closes it.
    h.get_all_by_label("Close").next().expect("a Close control").click();
    h.run_steps(2);
    assert!(!h.state().session.view.format_background_pane);
}

/// A cancelled web picker arrives as an empty inbox entry: Format Background's request ends with
/// it, so the picture dropped next inserts into the page instead of becoming the pane's draft.
#[test]
fn a_cancelled_background_picker_ends_its_request() {
    let mut png = Vec::new();
    image::RgbaImage::from_pixel(8, 8, image::Rgba([10, 200, 30, 255]))
        .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
        .unwrap();
    let inbox: crate::services::Inbox = Default::default();
    inbox.lock().unwrap().push((String::new(), Vec::new()));
    inbox.lock().unwrap().push(("dropped.png".into(), png));
    let mut a = WordApp::new(
        wordcraft_engine::Session::new(wordcraft_doc::Document::from_text("Background")),
        crate::Services { inbox: Some(inbox), ..Default::default() },
    );
    a.background_picture_pending = true;
    a.run("view.formatBackgroundPane", json!({"value": true})).unwrap();
    a.drain_inbox();
    assert!(!a.background_picture_pending, "the cancelled picker ended the request");
    assert!(!matches!(a.background.fill, crate::format_background::DraftFill::Picture { .. }), "the dropped picture isn't the pane's draft");
    assert!(!a.session.doc.media.is_empty(), "it went into the document");
    // A pane closed while its picker is in flight drops the request too.
    a.background_picture_pending = true;
    crate::format_background::forget(&mut a);
    assert!(!a.background_picture_pending);
}
