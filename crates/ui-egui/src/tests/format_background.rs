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

/// Two pictures of the same encoded length get different thumbnail cache keys: the head of a PNG
/// is its signature and header, so sampling only the head would show the first one's thumbnail.
#[test]
fn same_length_picture_data_gets_a_different_cache_key() {
    let key = |data: &Option<String>, media: &Option<String>| crate::format_background::picture_cache_key(&None, data, media, 7, |_| 11);
    let no_data: Option<String> = None;
    let no_media: Option<String> = None;
    let a = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8z8BQDwAEhQGAhKmMIQAAAABJRU5ErkJggg==".to_string();
    let b = a.replace("hKmMIQAAAABJRU5ErkJggg", "hKmMIQAAAABJRU5ErkJggH");
    assert_eq!(a.len(), b.len());
    assert_ne!(key(&Some(a.clone()), &no_media), key(&Some(b), &no_media), "the tails differ");
    assert_eq!(key(&Some(a.clone()), &no_media), key(&Some(a.clone()), &no_media), "the same data keeps its cache");
    // A byte outside the sampled head, middle and tail still changes the key: the whole data is
    // hashed, not three windows that two different pictures can share.
    let mut bytes = a.clone().into_bytes();
    bytes[30] = if bytes[30] == b'Z' { b'Y' } else { b'Z' };
    let interior = String::from_utf8(bytes).expect("base64 is ASCII");
    assert_eq!(interior.len(), a.len());
    assert_ne!(key(&Some(interior), &no_media), key(&Some(a.clone()), &no_media), "an interior byte differs");
    assert_eq!(key(&no_data, &Some("m.png".to_string())), Some("media:7:m.png:11".to_string()));
    // The media fallback is scoped: another document, or replaced bytes under one key, can't
    // serve the first document's cached texture.
    assert_ne!(
        key(&no_data, &Some("m.png".to_string())),
        crate::format_background::picture_cache_key(&None, &no_data, &Some("m.png".to_string()), 8, |_| 11),
        "another document gets its own cache"
    );
    assert_ne!(
        key(&no_data, &Some("m.png".to_string())),
        crate::format_background::picture_cache_key(&None, &no_data, &Some("m.png".to_string()), 7, |_| 12),
        "replaced bytes under one key get their own cache"
    );
}

/// Apply carries the document's whole stop list: a multi-stop gradient keeps its interior colours
/// and per-stop transparencies, and only the endpoint pickers and the slider change it.
#[test]
fn apply_keeps_a_multi_stop_gradient() {
    use wordcraft_doc::fill::{Gradient, GradientFlip, GradientKind};
    use wordcraft_doc::wordart::GradStop;
    let mut d = wordcraft_doc::Document::new();
    d.settings.page_gradient = Some(Gradient {
        stops: vec![
            GradStop { pos: 0.0, color: wordcraft_doc::Rgb(255, 0, 0), transparency: 10.0 },
            GradStop { pos: 50.0, color: wordcraft_doc::Rgb(0, 255, 0), transparency: 20.0 },
            GradStop { pos: 100.0, color: wordcraft_doc::Rgb(0, 0, 255), transparency: 30.0 },
        ],
        kind: GradientKind::Linear { angle: 90.0, scaled: false },
        tile: None,
        rot_with_shape: false,
        flip: GradientFlip::None,
    });
    let draft = crate::format_background::Draft::from_doc(&d);
    let params = draft.params();
    let stops = params["stops"].as_array().expect("the stops are passed, not only the endpoints");
    assert_eq!(stops.len(), 3);
    assert_eq!((stops[1]["color"].as_str(), stops[1]["transparency"].as_f64()), (Some("00FF00"), Some(20.0)));
    // Moving an endpoint's colour leaves the interior stop alone.
    let mut edited = draft.clone();
    if let crate::format_background::DraftFill::Gradient { start, .. } = &mut edited.fill {
        *start = wordcraft_doc::Rgb(1, 2, 3);
    }
    let params = edited.params();
    assert_eq!(params["stops"][0]["color"].as_str(), Some("010203"));
    assert_eq!(params["stops"][1]["color"].as_str(), Some("00FF00"), "the middle stop is untouched");
    // Moving the transparency slider moves every stop.
    if let crate::format_background::DraftFill::Gradient { transparency, .. } = &mut edited.fill {
        *transparency = 60.0;
    }
    let params = edited.params();
    assert!(params["stops"].as_array().is_some_and(|s| s.iter().all(|s| s["transparency"].as_f64() == Some(60.0))), "{params}");
}

/// The draft preview paints the stops Apply sends: the document's interior stops with the
/// endpoint pickers over them, not just the two endpoints.
#[test]
fn preview_paints_the_multi_stop_gradient_apply_sends() {
    use wordcraft_doc::wordart::GradStop;
    let stops = vec![
        GradStop { pos: 0.0, color: wordcraft_doc::Rgb(255, 0, 0), transparency: 0.0 },
        GradStop { pos: 50.0, color: wordcraft_doc::Rgb(0, 255, 0), transparency: 0.0 },
        GradStop { pos: 100.0, color: wordcraft_doc::Rgb(0, 0, 255), transparency: 0.0 },
    ];
    let painted = crate::format_background::Draft::gradient_stops(&stops, wordcraft_doc::Rgb(1, 2, 3), wordcraft_doc::Rgb(4, 5, 6), 0.0);
    assert_eq!(painted.len(), 3, "the interior stop survives");
    assert_eq!(
        (painted[0].color, painted[1].color, painted[2].color),
        (wordcraft_doc::Rgb(1, 2, 3), wordcraft_doc::Rgb(0, 255, 0), wordcraft_doc::Rgb(4, 5, 6))
    );
    // Fewer than two document stops fall back to the two endpoints.
    let fallback = crate::format_background::Draft::gradient_stops(&[], wordcraft_doc::Rgb::BLACK, wordcraft_doc::Rgb::WHITE, 25.0);
    assert_eq!(fallback.iter().map(|s| s.pos).collect::<Vec<_>>(), [0.0, 100.0]);
}
