use super::*;
use egui_kittest::kittest::Queryable;

fn window() -> H {
    let app = WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::new()), Default::default());
    let mut h = crate::test_harness::builder().with_size(vec2(1800.0, 900.0)).build_ui_state(
        |ui, app: &mut WordApp| {
            let ctx = ui.ctx().clone();
            app.logic(&ctx);
            app.ui(ui);
        },
        app,
    );
    steps(&mut h, 6);
    h
}

fn boundary(h: &mut H) -> Rect {
    let body = h.state_mut().session.layout().pages[0].body;
    let placed = &h.state().canvas.shown.pages[0];
    let scale = placed.scale as f32;
    Rect::from_min_size(placed.rect.min + vec2(body.x, body.y) * scale, vec2(body.w, body.h) * scale)
}

fn has_outline(h: &H, rect: Rect) -> bool {
    h.output().shapes.iter().any(|s| match &s.shape {
        egui::Shape::Rect(r) => r.rect.min.distance(rect.min) < 0.01 && r.rect.max.distance(rect.max) < 0.01 && r.stroke.width > 0.0,
        _ => false,
    })
}

#[test]
fn text_boundaries_outline_the_page_body_only_in_print_layout() {
    let mut h = window();
    let rect = boundary(&mut h);
    assert!(!has_outline(&h, rect));
    h.state_mut().run("view.textBoundaries", json!({"value": true})).unwrap();
    for zoom in [50, 100, 150] {
        h.state_mut().run("view.zoom", json!({"value": zoom})).unwrap();
        steps(&mut h, 4);
        let rect = boundary(&mut h);
        assert!(has_outline(&h, rect), "the guide follows the visible text area at {zoom}%");
    }
    for mode in ["view.draft", "view.webLayout", "view.outline", "view.readMode"] {
        let previous = boundary(&mut h);
        h.state_mut().run(mode, json!({})).unwrap();
        steps(&mut h, 4);
        let rect = if mode == "view.readMode" {
            assert!(h.state().canvas.shown.pages.is_empty(), "Read Mode uses its own page view");
            previous
        } else {
            boundary(&mut h)
        };
        assert!(!has_outline(&h, rect), "{mode} has no printed margin guide");
    }
    h.state_mut().run("view.printLayout", json!({})).unwrap();
    steps(&mut h, 4);
    let rect = boundary(&mut h);
    assert!(has_outline(&h, rect), "returning to Print Layout restores the guide");
}

#[test]
fn view_text_boundaries_checkbox_toggles_the_guides() {
    let mut h = window();
    h.state_mut().run("ui.tab", json!({"tab":"View"})).unwrap();
    steps(&mut h, 4);
    for enabled in [true, false] {
        h.get_by_label("Text Boundaries").click();
        steps(&mut h, 2);
        assert_eq!(h.state().session.view.text_boundaries, enabled);
        let rect = boundary(&mut h);
        assert_eq!(has_outline(&h, rect), enabled);
    }
}
