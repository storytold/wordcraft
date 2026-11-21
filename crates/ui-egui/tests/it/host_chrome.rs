//! A host that embeds WordCraft in its own window can turn off the app's title bar and File tab
//! (#4945).

use std::sync::Arc;

use craft::host::ScriptedFiles;
use egui_kittest::kittest::{NodeT as _, Queryable as _};
use serde_json::json;
use wordcraft_engine::Session;
use wordcraft_ui_egui::{Chrome, Services, WordApp};

/// The account button is named by the user's name.
const AUTHOR: &str = "Avery Quill";

fn harness_with(chrome: Chrome, files: Option<&Arc<ScriptedFiles>>) -> egui_kittest::Harness<'static, WordApp> {
    let mut app = WordApp::new(Session::new(wordcraft_doc::Document::from_text("one")), Services::default());
    if let Some(files) = files {
        let host = craft::host::Host::builder(wordcraft_ui_egui::APP).files(files.clone()).build();
        app = app.with_host(Arc::new(host));
    }
    // The labels below are English; an integration test otherwise follows the machine's language.
    app.ui.language = "en".into();
    app.session.author = AUTHOR.into();
    app.chrome = chrome;
    let mut h = crate::harness::builder().with_size(egui::vec2(1440.0, 900.0)).build_ui_state(
        |ui, app: &mut WordApp| {
            let ctx = ui.ctx().clone();
            app.logic(&ctx);
            app.ui(ui);
        },
        app,
    );
    frames(&mut h, 6);
    h
}

fn harness(chrome: Chrome) -> egui_kittest::Harness<'static, WordApp> {
    harness_with(chrome, None)
}

fn frames(h: &mut egui_kittest::Harness<'static, WordApp>, n: usize) {
    for _ in 0..n {
        crate::harness::frame(h);
    }
}

/// F10 (Alt) and then `key`, as a keyboard user picks a keytip: F opens File, 1 the toolbar's first button.
fn keytip(h: &mut egui_kittest::Harness<'static, WordApp>, key: egui::Key) {
    let press = |key| egui::Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers: egui::Modifiers::NONE };
    h.input_mut().events.push(press(egui::Key::F10));
    crate::harness::frame(h);
    h.input_mut().events.push(press(key));
    frames(h, 3);
}

#[test]
fn a_host_can_leave_out_the_title_bar_and_the_file_tab() {
    let full = harness(Chrome::default());
    let h = harness(Chrome { title_bar: false, file_tab: false });
    for label in ["AutoSave", AUTHOR, "File", "Share"] {
        assert!(full.query_by_label(label).is_some(), "{label} is drawn by default");
        assert!(h.query_by_label(label).is_none(), "{label} is left out");
    }
    let (with_bar, without) = (full.get_by_label("Italic").rect(), h.get_by_label("Italic").rect());
    assert!(without.min.y < with_bar.min.y, "the ribbon takes the title bar's room: {without:?} vs {with_bar:?}");
    let bar_off = harness(Chrome { title_bar: false, file_tab: true });
    assert!(bar_off.query_by_label("AutoSave").is_none(), "the title bar can go on its own");
    assert!(bar_off.query_by_label("File").is_some(), "and the File tab stays");
}

#[test]
fn without_the_title_bar_search_opens_as_a_dialog_that_takes_the_typing() {
    let mut h = harness(Chrome { title_bar: false, file_tab: true });
    // Help › Commands, with the document focused.
    h.state_mut().run("ui.dialog", json!({"name": "commands"})).unwrap();
    frames(&mut h, 3);
    h.input_mut().events.push(egui::Event::Text("needle".into()));
    frames(&mut h, 3);
    let focused = h.query_all(egui_kittest::kittest::By::new().role(egui::accesskit::Role::TextInput)).find(|n| n.accesskit_node().is_focused());
    assert_eq!(focused.and_then(|n| n.value()).as_deref(), Some("needle"), "the search box has the typing");
    assert_eq!(h.state().session.doc.plain_text(wordcraft_doc::StoryRef::Body), "one", "the document has none of it");
}

#[test]
fn without_the_title_bar_its_toolbar_has_no_keytips() {
    // The first button of the toolbar is Save; in a window with the title bar, F10, 1 presses it.
    let files = Arc::new(ScriptedFiles::new().in_browser().saving_suggested());
    let mut full = harness_with(Chrome::default(), Some(&files));
    keytip(&mut full, egui::Key::Num1);
    assert_eq!(files.requests().len(), 1, "F10, 1 saves with the toolbar shown");

    let files = Arc::new(ScriptedFiles::new().in_browser().saving_suggested());
    let mut h = harness_with(Chrome { title_bar: false, file_tab: true }, Some(&files));
    keytip(&mut h, egui::Key::Num1);
    assert!(files.requests().is_empty(), "the hidden toolbar's button is not pressed");
}

#[test]
fn without_the_file_tab_the_outlining_tab_still_comes_first() {
    let mut h = harness(Chrome { title_bar: true, file_tab: false });
    h.state_mut().run("view.outline", json!({})).unwrap();
    frames(&mut h, 3);
    let (outlining, home) = (h.get_by_label("Outlining").rect(), h.get_by_label("Home").rect());
    assert!(outlining.max.x <= home.min.x, "Outlining {outlining:?} comes before Home {home:?}");
}

#[test]
fn without_the_file_tab_the_backstage_view_never_opens() {
    // The keys reach File in a window that has it.
    let mut full = harness(Chrome::default());
    keytip(&mut full, egui::Key::F);
    assert!(full.state().ui.backstage, "F10, F opens File");

    let files = Arc::new(ScriptedFiles::new().in_browser().saving_suggested());
    let mut h = harness_with(Chrome { title_bar: true, file_tab: false }, Some(&files));
    keytip(&mut h, egui::Key::F);
    assert!(!h.state().ui.backstage, "F10, F has no File tab to open");
    let app = h.state_mut();
    assert!(!app.start_screen(), "no Start screen, which is File › Home");
    assert!(app.has_document(), "the window keeps its document");
    let (page, pane) = (app.ui.backstage_page.clone(), app.ui.options_pane.clone());
    for (id, params) in [
        ("ui.tab", json!({"tab": "File"})),
        ("ui.backstage", json!({"value": true, "page": "options", "pane": "language"})),
        ("ui.backstage", json!({})),
        ("file.print", json!({})),
        ("file.options", json!({})),
        ("file.recover", json!({})),
    ] {
        app.run(id, params.clone()).unwrap();
        assert!(!app.ui.backstage, "{id} {params} leaves File closed");
        assert_eq!((&app.ui.backstage_page, &app.ui.options_pane), (&page, &pane), "{id} {params} keeps the page File opens on");
    }
    // Save (Ctrl+S) still hands the document over: here a browser download.
    app.run("file.save", json!({})).unwrap();
    let name = files.requests().last().and_then(|r| r.file_name.clone()).expect("the save asked for a name");
    assert!(files.file(std::path::Path::new(&name)).is_some(), "the save handed over its bytes");
    frames(&mut h, 3);
    assert!(!h.state().ui.backstage);
    assert!(h.query_by_label("AutoSave").is_some(), "the title bar can stay on its own");
    assert!(h.query_by_label(AUTHOR).is_none(), "without its account button, which opens File › Options");
    assert!(h.query_by_label("Share").is_none(), "Share, which opens File › Export, goes with the tab");
}

#[test]
fn turning_the_file_tab_off_closes_an_open_backstage() {
    let mut h = harness(Chrome::default());
    keytip(&mut h, egui::Key::F);
    assert!(h.state().ui.backstage, "File opens with the tab on");
    h.state_mut().chrome.file_tab = false;
    frames(&mut h, 3);
    assert!(!h.state().ui.backstage, "and closes when the host turns the tab off");
}
