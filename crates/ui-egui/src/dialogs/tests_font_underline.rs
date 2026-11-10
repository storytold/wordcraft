//! Font dialog: Word's Underline style and Underline color boxes, and Double strikethrough.

use super::*;
use egui::accesskit::Role;
use egui_kittest::kittest::Queryable;
use wordcraft_doc::Rgb;
use wordcraft_doc::props::{CharProps, Underline};
use wordcraft_engine::Session;

/// `text` with every character selected.
fn selected(text: &str) -> WordApp {
    let mut app = WordApp::new(Session::new(wordcraft_doc::Document::new()), crate::Services::default());
    let _ = app.run("document.setText", json!({"text": text}));
    let _ = app.run("select.all", json!({}));
    app
}

fn char_at(app: &WordApp, i: usize) -> CharProps {
    app.session.doc.para_at(&wordcraft_doc::Pos::body(0, 0)).map(|p| p.props_of_char(i).clone()).unwrap_or_default()
}

fn opened(app: &mut WordApp) -> Dialog {
    let Some(d) = Dialog::open("font", app) else { panic!("no font dialog") };
    d
}

/// `format.state` names the selection's underline style, and says whether it is all double struck.
#[test]
fn format_state_reports_underline_style_and_double_strike() {
    let mut app = selected("Example");
    let _ = app.run("format.set", json!({"props": {"underline": "dotDash", "doubleStrike": true}}));
    let st = app.session.run("format.state", &json!({})).unwrap_or_default();
    assert_eq!(st.get("underlineStyle"), Some(&json!("dotDash")));
    assert_eq!(st.get("doubleStrike"), Some(&json!(true)));
    assert_eq!(st.get("underline"), Some(&json!(true)), "the plain flag stays");
}

/// The dialog opens with the text's own underline style and colour and its double strikethrough.
#[test]
fn font_dialog_shows_the_underline_style_colour_and_double_strike() {
    let mut app = selected("Example");
    let _ = app.run("format.set", json!({"props": {"underline": "double", "doubleStrike": true}}));
    let _ = app.run("format.set", json!({"props": {"underlineColor": "C00000"}}));
    assert_eq!(char_at(&app, 0).underline_color, Some(Rgb(0xC0, 0, 0)), "setup: red underline");
    let props = font_props(&opened(&mut app));
    assert_eq!(props.get("underline"), Some(&json!("double")));
    assert_eq!(props.get("underlineColor"), Some(&json!("C00000")));
    assert_eq!(props.get("doubleStrike"), Some(&json!(true)));
}

fn harness(app: WordApp) -> egui_kittest::Harness<'static, WordApp> {
    egui_kittest::Harness::builder().with_size(egui::vec2(1440.0, 900.0)).build_ui_state(
        |ui, app: &mut WordApp| {
            let ctx = ui.ctx().clone();
            app.logic(&ctx);
            app.ui(ui);
        },
        app,
    )
}

/// Choosing a style in the Underline style box and pressing OK applies that style, not a single
/// underline, in one undo step.
#[test]
fn font_dialog_applies_each_underline_style() {
    for (entry, u) in [
        ("Words only", Underline::Words),
        ("Double", Underline::Double),
        ("Thick", Underline::Thick),
        ("Dotted", Underline::Dotted),
        ("Wave", Underline::Wave),
        ("(none)", Underline::None),
    ] {
        let mut app = selected("Example");
        let _ = app.run("format.set", json!({"props": {"underline": "dash"}}));
        app.dialog = Dialog::open("font", &mut app);
        let depth = app.session.undo_depth();
        let mut h = harness(app);
        h.run_steps(4);
        h.get_by_role_and_label(Role::ComboBox, "Underline style:").click();
        h.run_steps(2);
        h.get_by_label(entry).click();
        h.run_steps(3);
        h.get_by_role_and_label(Role::Button, "OK").click();
        h.run_steps(4);
        assert_eq!(char_at(h.state(), 0).underline, Some(u), "{entry}");
        assert_eq!(h.state().session.undo_depth(), depth + 1, "one undo step, {entry}");
    }
}

/// Guard: OK after changing only the size keeps a red underline red.
#[test]
fn font_dialog_keeps_the_underline_colour() {
    let mut app = selected("Example");
    let _ = app.run("format.set", json!({"props": {"underline": "single"}}));
    let _ = app.run("format.set", json!({"props": {"underlineColor": "C00000"}}));
    let mut d = opened(&mut app);
    if let Dialog::Font { size, .. } = &mut d {
        *size = "14".into();
    }
    apply_font(&mut app, &d);
    let c = char_at(&app, 0);
    assert_eq!((c.underline, c.underline_color, c.size), (Some(Underline::Single), Some(Rgb(0xC0, 0, 0)), Some(14.0)));
}

/// Ticking Double strikethrough clears Strikethrough, as in Word, and OK applies it.
#[test]
fn font_dialog_double_strikethrough_replaces_strikethrough() {
    let mut app = selected("Example");
    let _ = app.run("format.set", json!({"props": {"strike": true}}));
    app.dialog = Dialog::open("font", &mut app);
    let mut h = harness(app);
    h.run_steps(4);
    h.get_by_label("Double strikethrough").click();
    h.run_steps(3);
    h.get_by_role_and_label(Role::Button, "OK").click();
    h.run_steps(4);
    let c = char_at(h.state(), 0);
    assert_eq!(c.double_strike, Some(true));
    assert_ne!(c.strike, Some(true), "strikethrough is off");
}

/// Guard: text whose runs have different underline styles keeps them when OK changes only the size.
#[test]
fn font_dialog_leaves_mixed_underline_styles_alone() {
    let mut app = selected("Alpha beta");
    let _ = app.run("format.set", json!({"props": {"underline": "double"}}));
    let _ = app.run("select.text", json!({"text": "beta"}));
    let _ = app.run("format.set", json!({"props": {"underline": "wave"}}));
    let _ = app.run("select.all", json!({}));
    let mut d = opened(&mut app);
    if let Dialog::Font { size, .. } = &mut d {
        *size = "15".into();
    }
    apply_font(&mut app, &d);
    assert_eq!((char_at(&app, 0).underline, char_at(&app, 0).size), (Some(Underline::Double), Some(15.0)));
    assert_eq!((char_at(&app, 7).underline, char_at(&app, 7).size), (Some(Underline::Wave), Some(15.0)));
}
