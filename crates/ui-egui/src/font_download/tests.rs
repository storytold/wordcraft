use egui_kittest::kittest::Queryable;
use serde_json::json;
use wordcraft_engine::Session;

use super::*;

/// Aptos is installed on this machine: nothing is missing, so there is nothing to tell.
fn aptos_installed() -> bool {
    wordcraft_fonts::FontDb::global().families().iter().any(|f| f.eq_ignore_ascii_case("Aptos"))
}

/// A document with text in Aptos (which Microsoft gives away) and in a font nobody has,
/// opened from a file when `opened`.
fn app(opened: bool) -> WordApp {
    let mut a = WordApp::new(Session::new(wordcraft_doc::Document::new()), crate::Services::default());
    a.session.run("format.font", &json!({"name": "Aptos"})).unwrap();
    a.session.run("text.insert", &json!({"text": "Lanterns"})).unwrap();
    a.session.run("text.newParagraph", &json!({})).unwrap();
    a.session.run("format.font", &json!({"name": "Wordcraft Nowhere Face"})).unwrap();
    a.session.run("text.insert", &json!({"text": "Harbour"})).unwrap();
    if opened {
        a.session.path = Some("letter.docx".into());
    }
    a.tell_about_font_downloads();
    a
}

fn harness(app: WordApp) -> egui_kittest::Harness<'static, WordApp> {
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

fn shown(h: &egui_kittest::Harness<'static, WordApp>) -> Option<&'static str> {
    h.state().dialog.as_ref().map(Dialog::name)
}

#[test]
fn an_opened_document_missing_a_downloadable_font_is_told_about_once() {
    if aptos_installed() {
        return;
    }
    let mut h = harness(app(true));
    let Some(Dialog::FontDownload { fonts, dont_show }) = h.state().dialog.clone() else { panic!("no box: {:?}", shown(&h)) };
    // Only the font that can be downloaded, with what it is drawn in and Microsoft's page.
    assert_eq!(fonts.len(), 1, "{fonts:?}");
    assert_eq!((fonts[0].font.as_str(), fonts[0].publisher.as_str()), ("Aptos", "Microsoft"));
    assert!(fonts[0].url.starts_with("https://www.microsoft.com/"), "{}", fonts[0].url);
    assert!(!fonts[0].shown_as.is_empty() && !fonts[0].shown_as.iter().any(|f| f == "Aptos"), "{:?}", fonts[0].shown_as);
    assert!(!dont_show);
    // The box says so: the font, its substitute, the link, how to install, and the opt-out.
    assert!(h.query_by_label("Aptos").is_some());
    assert!(h.query_by_label(fonts[0].shown_as.join(", ").as_str()).is_some());
    assert!(h.query_by_label("Download from Microsoft").is_some());
    assert!(h.query_by_label_contains("Add Font… below").is_some());
    assert!(h.query_by_label("Don't show this again").is_some());
    // Closed, it stays closed for this document.
    // The box's own Close button, below the title bar's.
    let close =
        h.query_all_by_role_and_label(egui::accesskit::Role::Button, "Close").max_by(|a, b| a.rect().center().y.total_cmp(&b.rect().center().y));
    close.expect("a Close button").click();
    h.run_steps(4);
    assert_eq!(shown(&h), None);
    assert!(h.state().ui.font_download_notice, "closing alone keeps the box on");
}

#[test]
fn the_box_comes_up_only_where_the_host_asked_for_it() {
    let mut a = app(true);
    a.font_notices = false;
    let h = harness(a);
    assert_eq!(shown(&h), None, "headless: tests and ui_shot");
}

#[test]
fn a_new_document_is_not_told_about() {
    let h = harness(app(false));
    assert_eq!(shown(&h), None, "its fonts are WordCraft's choice");
}

#[test]
fn dont_show_this_again_turns_the_box_off() {
    if aptos_installed() {
        return;
    }
    let mut h = harness(app(true));
    assert_eq!(shown(&h), Some("fontDownload"));
    h.get_by_label("Don't show this again").click();
    h.run_steps(2);
    assert!(!h.state().ui.font_download_notice, "kept at once");
    // Saved between runs, and read back.
    let saved = serde_json::to_value(h.state().prefs()).unwrap();
    assert_eq!(saved["fontDownloadNotice"], json!(false), "{saved}");
    // The next opened document isn't told about.
    let mut a = app(true);
    a.ui.font_download_notice = false;
    let h = harness(a);
    assert_eq!(shown(&h), None);
}

#[test]
fn the_box_waits_for_an_open_dialog_and_can_be_opened_by_hand() {
    if aptos_installed() {
        return;
    }
    let mut a = app(true);
    a.dialog = Dialog::open("wordCount", &mut a);
    let mut h = harness(a);
    assert_eq!(shown(&h), Some("wordCount"), "the open dialog stays");
    h.state_mut().dialog = None;
    h.run_steps(3);
    assert_eq!(shown(&h), Some("fontDownload"), "then the box");
    h.state_mut().dialog = None;
    h.run_steps(3);
    assert_eq!(shown(&h), None);
    // The status bar's menu (`ui.dialog`) opens it again, even with the box turned off.
    h.state_mut().ui.font_download_notice = false;
    h.state_mut().run("ui.dialog", json!({"name": "fontDownload"})).unwrap();
    let Some(Dialog::FontDownload { fonts, dont_show }) = h.state().dialog.clone() else { panic!("no box") };
    assert_eq!(fonts.len(), 1);
    assert!(dont_show, "the box shows the setting");
}

#[test]
fn a_document_a_script_opened_is_not_told_about() {
    let mut a = app(true);
    let id = a.session.document_id();
    skip(&mut a, id);
    skip(&mut a, id);
    assert_eq!(a.font_notice_seen, vec![id]);
    let h = harness(a);
    assert_eq!(shown(&h), None);
}

#[test]
fn a_document_with_nothing_to_download_says_so_when_asked() {
    let mut a = WordApp::new(Session::new(wordcraft_doc::Document::from_text("Lanterns")), crate::Services::default());
    a.session.run("select.all", &json!({})).unwrap();
    a.session.run("format.font", &json!({"name": "Wordcraft Nowhere Face"})).unwrap();
    a.session.path = Some("letter.docx".into());
    a.tell_about_font_downloads();
    let mut h = harness(a);
    assert_eq!(shown(&h), None, "a font nobody gives away brings up no box");
    h.state_mut().run("ui.dialog", json!({"name": "fontDownload"})).unwrap();
    h.run_steps(3);
    assert!(h.query_by_label_contains("None of the fonts").is_some());
}

#[test]
fn only_so_many_documents_are_remembered() {
    let mut a = app(false);
    for id in 0..(REMEMBERED as u64 + 50) {
        skip(&mut a, id);
    }
    assert_eq!(a.font_notice_seen.len(), REMEMBERED);
    assert_eq!(a.font_notice_seen.last(), Some(&(REMEMBERED as u64 + 49)));
}
