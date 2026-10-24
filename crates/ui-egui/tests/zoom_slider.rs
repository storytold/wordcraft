//! The status bar's zoom slider: Word's scale, and the wheel over it zooms 10% a notch (#2073).

use wordcraft_engine::Session;
use wordcraft_ui_egui::{Services, WordApp};

/// A WordCraft window with a document open.
fn harness() -> egui_kittest::Harness<'static, WordApp> {
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
    h
}

/// The zoom percentage the canvas is at, rounded like the status bar shows it.
fn zoom_percent(h: &egui_kittest::Harness<'static, WordApp>) -> f32 {
    (h.state().canvas.scale / wordcraft_ui_egui::canvas::PX_PER_PT * 100.0).round()
}

/// The status bar's zoom slider rect.
fn slider_rect(h: &mut egui_kittest::Harness<'static, WordApp>) -> egui::Rect {
    use egui_kittest::kittest::Queryable;
    h.query_all(egui_kittest::kittest::By::new().role(egui::accesskit::Role::Slider))
        .next()
        .expect("the status bar's zoom slider is in the accessibility tree")
        .rect()
}

/// A mouse wheel notch (one line) over the slider.
fn wheel_notch(h: &mut egui_kittest::Harness<'static, WordApp>, lines: f32) {
    h.event(egui::Event::MouseWheel {
        unit: egui::MouseWheelUnit::Line,
        delta: egui::vec2(0.0, lines),
        phase: egui::TouchPhase::Move,
        modifiers: egui::Modifiers::NONE,
    });
    h.step();
}

#[test]
fn a_wheel_notch_over_the_slider_zooms_by_ten_percent() {
    let mut h = harness();
    let rect = slider_rect(&mut h);
    h.hover_at(rect.center());
    h.step();

    let before = zoom_percent(&h);
    wheel_notch(&mut h, 1.0);
    assert_eq!(zoom_percent(&h), (before + 10.0).min(500.0), "scrolling up zooms in");
    wheel_notch(&mut h, -1.0);
    assert_eq!(zoom_percent(&h), before, "scrolling down zooms back out");
    wheel_notch(&mut h, -1.0);
    assert_eq!(zoom_percent(&h), (before - 10.0).max(10.0), "scrolling down zooms out");
}

#[test]
fn wheel_zoom_stays_inside_the_track() {
    let mut h = harness();
    let rect = slider_rect(&mut h);
    h.hover_at(rect.center());
    h.step();
    for _ in 0..60 {
        wheel_notch(&mut h, 1.0);
    }
    assert_eq!(zoom_percent(&h), 500.0, "zoom stops at the track's end");
    for _ in 0..120 {
        wheel_notch(&mut h, -1.0);
    }
    assert_eq!(zoom_percent(&h), 10.0, "zoom stops at the track's start");
}
