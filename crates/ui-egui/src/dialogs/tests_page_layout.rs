//! Page Setup: Gutter, Section start and Header/Footer From edge, as in Word.

use super::*;
use egui::accesskit::Role;
use egui_kittest::kittest::Queryable;
use wordcraft_doc::section::{SectionProps, SectionStart};
use wordcraft_engine::Session;

fn app_with(text: &str) -> WordApp {
    let mut app = WordApp::new(Session::new(wordcraft_doc::Document::new()), crate::Services::default());
    let _ = app.run("document.setText", json!({"text": text}));
    app
}

fn sect(app: &WordApp) -> SectionProps {
    wordcraft_engine::cmd::page::sect(&app.session)
}

/// Page Setup open on `app`'s caret section, drawn.
fn opened(mut app: WordApp) -> egui_kittest::Harness<'static, WordApp> {
    app.dialog = Dialog::open("pageSetup", &mut app);
    let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(1440.0, 900.0)).build_ui_state(
        |ui, app: &mut WordApp| {
            let ctx = ui.ctx().clone();
            app.logic(&ctx);
            app.ui(ui);
        },
        app,
    );
    h.run_steps(4);
    h
}

/// Types `pt` points, in the measurement unit the dialog shows, into the number box `label`.
fn type_length(h: &mut egui_kittest::Harness<'static, WordApp>, label: &str, pt: f32) {
    let unit = crate::options::unit().pt_per_unit();
    h.get_by_role_and_label(Role::SpinButton, label).click();
    h.run_steps(2);
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
    h.event(egui::Event::Paste(format!("{}", pt / unit)));
    h.run_steps(2);
    h.key_press(egui::Key::Tab);
    h.run_steps(2);
}

fn ok(h: &mut egui_kittest::Harness<'static, WordApp>) {
    h.get_by_role_and_label(Role::Button, "OK").click();
    h.run_steps(4);
    assert!(h.state().dialog.is_none(), "OK closes the dialog");
}

/// Engine: `layout.margins` takes the header and footer distances; `layout.sectionStart` the break type.
#[test]
fn engine_sets_header_footer_distance_and_section_start() {
    let mut app = app_with("Text");
    app.run("layout.margins", json!({"header": 72, "footer": 54, "gutter": 18})).unwrap();
    let s = sect(&app);
    assert_eq!((s.header, s.footer, s.gutter), (72.0, 54.0, 18.0));
    app.run("layout.sectionStart", json!({"value": "oddPage"})).unwrap();
    assert_eq!(sect(&app).start, SectionStart::OddPage);
    app.run("layout.sectionStart", json!({"value": "continuous"})).unwrap();
    assert_eq!(sect(&app).start, SectionStart::Continuous);
    assert!(app.run("layout.sectionStart", json!({"value": "sideways"})).is_err(), "unknown value refused");
    assert_eq!(sect(&app).start, SectionStart::Continuous);
}

/// A fresh document opens with Word's defaults: Gutter 0, Header and Footer 0.5 in, New page.
#[test]
fn page_setup_shows_gutter_header_footer_and_section_start() {
    let h = opened(app_with("Text"));
    h.get_by_role_and_label(Role::SpinButton, "Gutter:");
    h.get_by_role_and_label(Role::SpinButton, "Header:");
    h.get_by_role_and_label(Role::SpinButton, "Footer:");
    assert_eq!(h.get_by_role_and_label(Role::ComboBox, "Section start:").value().as_deref(), Some("New page"));
}

#[test]
fn page_setup_sets_the_gutter() {
    let app = app_with("Text");
    let width = sect(&app).text_width();
    let depth = app.session.undo_depth();
    let mut h = opened(app);
    type_length(&mut h, "Gutter:", 36.0);
    ok(&mut h);
    let s = sect(h.state());
    assert!((s.gutter - 36.0).abs() < 0.05, "gutter {}", s.gutter);
    assert!((s.text_width() - (width - 36.0)).abs() < 0.1, "the text is 36 pt narrower");
    assert_eq!(h.state().session.undo_depth(), depth + 1, "one undo step");
}

#[test]
fn page_setup_sets_header_and_footer_from_edge() {
    let mut h = opened(app_with("Text"));
    type_length(&mut h, "Header:", 72.0);
    type_length(&mut h, "Footer:", 54.0);
    ok(&mut h);
    let s = sect(h.state());
    assert!((s.header - 72.0).abs() < 0.05 && (s.footer - 54.0).abs() < 0.05, "header {} footer {}", s.header, s.footer);
}

#[test]
fn page_setup_sets_the_section_start() {
    for (entry, want) in [
        ("Odd page", SectionStart::OddPage),
        ("Even page", SectionStart::EvenPage),
        ("Continuous", SectionStart::Continuous),
        ("New column", SectionStart::NextColumn),
    ] {
        let mut h = opened(app_with("Text"));
        h.get_by_role_and_label(Role::ComboBox, "Section start:").click();
        h.run_steps(2);
        h.get_by_label(entry).click();
        h.run_steps(3);
        ok(&mut h);
        assert_eq!(sect(h.state()).start, want, "{entry}");
    }
}

/// A section that starts on an odd page opens showing it, and an unchanged OK keeps it all.
#[test]
fn page_setup_unchanged_ok_keeps_the_layout_settings() {
    let mut app = app_with("Text");
    app.run("layout.margins", json!({"gutter": 18})).unwrap();
    let mut s = sect(&app);
    s.start = SectionStart::OddPage;
    s.header = 30.0;
    app.run("layout.pageSetup", json!({"section": s})).unwrap();
    let was = sect(&app);
    let depth = app.session.undo_depth();
    let mut h = opened(app);
    assert_eq!(h.get_by_role_and_label(Role::ComboBox, "Section start:").value().as_deref(), Some("Odd page"));
    ok(&mut h);
    assert_eq!(sect(h.state()), was);
    assert_eq!(h.state().session.undo_depth(), depth, "no undo step");
}
