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
    harness_sized(app, egui::vec2(1440.0, 900.0))
}

fn harness_sized(app: WordApp, size: egui::Vec2) -> egui_kittest::Harness<'static, WordApp> {
    let mut h = egui_kittest::Harness::builder().with_size(size).build_ui_state(
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
    // The box says so: the font, its substitute and publisher, the download, how to add it,
    // and the opt-out.
    // The headline counts what can be downloaded, not every font the document is missing.
    assert!(h.query_by_label("1 missing font can be downloaded").is_some());
    assert!(h.query_by_label_contains("isn't installed").is_none(), "other fonts may be missing too");
    assert!(h.query_by_label("Aptos").is_some());
    assert!(h.query_by_label(format!("Shown as {} · Microsoft", fonts[0].shown_as.join(", ")).as_str()).is_some());
    assert!(h.query_by_label_contains("following them is up to you").is_some(), "whose part the licence is");
    assert!(h.query_by_label_contains("limits who may use it").is_none(), "Aptos has no such limit");
    assert!(h.query_by_label("Pick the font files:").is_some());
    assert!(h.query_by_role_and_label(egui::accesskit::Role::Button, "Add Font…").is_some());
    assert!(h.query_by_label("Don't show this again").is_some());
    // Download opens the publisher's page in the browser; nothing is downloaded here.
    h.get_by_role_and_label(egui::accesskit::Role::Button, "Download").click();
    let mut opened = false;
    for _ in 0..4 {
        h.step();
        opened |= h.output().platform_output.commands.iter().any(|c| matches!(c, egui::OutputCommand::OpenUrl(u) if u.url == fonts[0].url));
    }
    assert!(opened, "the page is opened");
    assert_eq!(shown(&h), Some("fontDownload"), "and the box stays for the next step");
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

/// Fonts that come in one download share a card and a Download button.
#[test]
fn fonts_of_one_download_are_listed_together() {
    let row = |font: &str, shown: &str, url: &str| Row {
        font: font.into(),
        shown_as: vec![shown.into()],
        download: font.into(),
        publisher: "Microsoft".into(),
        url: url.into(),
        restricted: false,
    };
    let fonts =
        [row("Aptos", "Carlito", "https://a"), row("Cascadia Code", "Liberation Mono", "https://c"), row("Aptos Display", "Carlito", "https://a")];
    let g = groups(&fonts);
    assert_eq!(g.len(), 2);
    assert_eq!(
        (g[0].fonts.as_slice(), g[0].shown_as.as_slice(), g[0].url),
        (["Aptos", "Aptos Display"].as_slice(), ["Carlito"].as_slice(), "https://a")
    );
    assert_eq!((g[1].fonts.as_slice(), g[1].url), (["Cascadia Code"].as_slice(), "https://c"));
}

/// As many downloads as the box lists, in a short window and a very short one: what is above
/// the row with Close scrolls, so the opt-out and Close stay on screen and the box in the window.
#[test]
fn many_downloads_in_a_short_window_leave_the_opt_out_and_close_on_screen() {
    for height in [600.0, 400.0] {
        let mut a = WordApp::new(Session::new(wordcraft_doc::Document::from_text("Lanterns")), crate::Services::default());
        let fonts: Vec<Row> = (0..LISTED)
            .map(|i| Row {
                font: format!("Harbour Face {i}"),
                shown_as: vec!["Carlito".into()],
                download: format!("Harbour Face {i}"),
                publisher: "Lantern Works".into(),
                url: format!("https://example.org/{i}"),
                restricted: false,
            })
            .collect();
        a.dialog = Some(Dialog::FontDownload { fonts, dont_show: false });
        let h = harness_sized(a, egui::vec2(1000.0, height));
        // The steps are still there, to scroll to.
        assert!(h.query_by_label("Add Font…").is_some());
        for label in ["Don't show this again", "Close"] {
            // Close is also the window's close box: every match is on screen.
            let rects: Vec<egui::Rect> = h.query_all_by_label(label).map(|n| n.rect()).collect();
            assert!(!rects.is_empty(), "no {label}");
            for r in rects {
                assert!(r.min.y >= 0.0 && r.max.y <= height, "{label} is at {r:?}, outside the {height} pt window");
            }
        }
    }
}

/// A font whose publisher limits its use says so on its card, and has a card of its own.
#[test]
fn a_font_with_a_limited_licence_says_so() {
    let mut a = WordApp::new(Session::new(wordcraft_doc::Document::from_text("Lanterns")), crate::Services::default());
    a.session.run("select.all", &json!({})).unwrap();
    a.session.run("format.font", &json!({"name": "Segoe UI Variable"})).unwrap();
    let layout = a.session.layout();
    if !wordcraft_engine::cmd::font::missing_fonts(&a.session.doc, &layout).iter().any(|f| f.font == "Segoe UI Variable") {
        return; // Installed here: not missing.
    }
    a.dialog = Dialog::open("fontDownload", &mut a);
    let Some(Dialog::FontDownload { fonts, .. }) = a.dialog.clone() else { panic!("no box") };
    assert!(fonts.iter().any(|f| f.font == "Segoe UI Variable" && f.restricted), "{fonts:?}");
    let h = harness(a);
    assert!(h.query_by_label_contains("limits who may use it").is_some());
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
