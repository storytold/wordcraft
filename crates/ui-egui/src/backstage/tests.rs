use std::sync::Arc;

use craft::host::{Picked, ScriptedFiles};
use egui_kittest::kittest::Queryable;
use serde_json::json;

use crate::{Services, WordApp};

/// A window with an empty document.
fn new_app() -> WordApp {
    WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::new()), Services::default())
}

/// A window whose file service is `files`: every pick and save of the page goes to it.
fn app_with_files(files: &Arc<ScriptedFiles>) -> WordApp {
    let host = craft::host::Host::builder(crate::APP).files(files.clone()).build();
    new_app().with_host(Arc::new(host))
}

/// The app in a window of `size`, run until it settles.
fn harness(a: WordApp, size: egui::Vec2) -> egui_kittest::Harness<'static, WordApp> {
    let mut h = crate::test_harness::builder().with_size(size).build_ui_state(
        |ui, app: &mut WordApp| {
            let ctx = ui.ctx().clone();
            app.logic(&ctx);
            app.ui(ui);
        },
        a,
    );
    h.run_steps(4);
    h
}

/// A 1440 × 900 window showing File on `page`.
fn file_page(mut a: WordApp, page: &str) -> egui_kittest::Harness<'static, WordApp> {
    a.run("ui.backstage", json!({"value": true, "page": page})).unwrap();
    harness(a, egui::vec2(1440.0, 900.0))
}

/// A recent row by its file name. The framework names a row with its folder after the file name
/// (`name` then the path), so a row is found by that prefix, not by the exact label.
fn recent_row<'a>(h: &'a egui_kittest::Harness<'static, WordApp>, name: &'a str) -> Option<egui_kittest::Node<'a>> {
    h.query_all_by_label_contains(name).next()
}

/// #1714: every side-menu item has its own icon, and each icon is one the app registers or the
/// framework catalogue has (an unknown name would draw nothing useful).
#[test]
fn every_side_menu_item_has_a_drawn_icon() {
    let mut names = std::collections::HashSet::new();
    for (id, _, icon) in super::PAGES {
        assert!(names.insert(icon), "{id}: {icon} is used twice");
        let app_has = crate::icons::WORDCRAFT_ICONS.icons.iter().any(|(name, _)| *name == icon);
        let catalogue_has = craft::ui::primitives::icon_names().contains(&icon);
        assert!(app_has || catalogue_has, "{id}: {icon} is in neither the app's icon set nor the catalogue");
    }
}

/// #2938: opening a deleted file from Recent keeps File open and the page shows the error instead
/// of closing it on a greyed-out window.
#[test]
#[cfg(not(target_arch = "wasm32"))]
fn opening_a_missing_recent_file_keeps_file_open_with_the_error() {
    let missing = std::env::temp_dir().join(format!("wordcraft-missing-{}.docx", std::process::id()));
    let name = missing.file_name().unwrap().to_string_lossy().into_owned();
    let mut a = new_app();
    a.ui.recent = vec![missing.to_string_lossy().into_owned()];
    let mut h = file_page(a, "open");
    recent_row(&h, name.as_str()).expect("the missing file's row shows").focus();
    h.run_steps(2);
    h.key_press(egui::Key::Enter);
    h.run_steps(4);
    assert!(h.state().ui.backstage, "File stays open");
    let err = h.state().backstage_state.open_error.clone().expect("the page kept the error");
    assert!(!err.is_empty());
    h.get_by_label(err.as_str());
}

#[cfg(not(target_arch = "wasm32"))]
fn recovery_fixture() -> (WordApp, std::path::PathBuf) {
    let root = std::env::temp_dir().join(format!(
        "wordcraft-recovery-ui-{}-{}",
        std::process::id(),
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()
    ));
    let id = "0123456789abcdef0123456789abcdef";
    let dir = root.join(id);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("snapshot-00000000000000000001.json"),
        serde_json::to_vec(&json!({
            "version": 1, "generation": 1, "info": {"id": id, "name": "Synthetic draft", "savedAt": "2026-10-11T10:42:00Z"},
            "document": wordcraft_doc::Document::from_text("Recovered synthetic work")
        }))
        .unwrap(),
    )
    .unwrap();
    let mut app = new_app();
    app.ui.language = "en".into();
    (app, root)
}

#[cfg(not(target_arch = "wasm32"))]
fn recovery_app() -> (WordApp, std::path::PathBuf) {
    let (mut app, root) = recovery_fixture();
    app.session.enable_recovery(root.clone(), std::sync::Arc::new(|| {})).unwrap();
    (app, root)
}

/// Hold discovery polling until the accessible control has dispatched its command.
/// Warming up first keeps font installation independent of worker timing.
#[cfg(not(target_arch = "wasm32"))]
fn pending_recovery_harness(app: WordApp, root: &std::path::Path) -> (egui_kittest::Harness<'static, WordApp>, std::rc::Rc<std::cell::Cell<bool>>) {
    let poll = std::rc::Rc::new(std::cell::Cell::new(true));
    let gate = poll.clone();
    let mut h = crate::test_harness::builder().with_size(egui::vec2(1440.0, 900.0)).build_ui_state(
        move |ui, app: &mut WordApp| {
            if gate.get() {
                app.logic(&ui.ctx().clone());
            }
            app.ui(ui);
        },
        app,
    );
    h.run_steps(4);
    poll.set(false);
    h.state_mut().session.enable_recovery(root.to_path_buf(), std::sync::Arc::new(|| {})).unwrap();
    assert!(!h.state().session.recovery_ready());
    (h, poll)
}

#[cfg(not(target_arch = "wasm32"))]
fn recovery_harness(app: WordApp) -> egui_kittest::Harness<'static, WordApp> {
    crate::test_harness::builder().with_size(egui::vec2(1440.0, 900.0)).build_ui_state(
        |ui, app: &mut WordApp| {
            let ctx = ui.ctx().clone();
            app.logic(&ctx);
            app.ui(ui);
        },
        app,
    )
}

#[cfg(not(target_arch = "wasm32"))]
fn settle_recovery(h: &mut egui_kittest::Harness<'_, WordApp>) {
    let end = std::time::Instant::now() + std::time::Duration::from_secs(5);
    loop {
        h.run_steps(2);
        if h.state().session.recovery_ready() && !h.state().session.recovery_busy() {
            break;
        }
        assert!(std::time::Instant::now() < end);
        std::thread::sleep(std::time::Duration::from_millis(2));
    }
    // Worker completion is a state boundary; egui needs subsequent frames to finish
    // font installation/layout and expose the resulting buttons to AccessKit.
    h.run_steps(4);
}

#[test]
#[cfg(not(target_arch = "wasm32"))]
fn recovery_does_not_replace_an_explicit_new_document() {
    for template in ["Blank document", "Report"] {
        let (mut app, root) = recovery_fixture();
        app.ui.backstage = true;
        // The Info page has no New or Open button of its own, so the names below are the side menu's.
        app.ui.backstage_page = "info".into();
        let (mut h, poll) = pending_recovery_harness(app, &root);
        let previous = h.state().session.document_id();
        h.get_by_label("New").click();
        h.run_steps(4);
        h.get_by_label(template).click();
        h.run_steps(4);
        assert!(!h.state().session.recovery_ready());
        let document = h.state().session.document_id();
        let text = h.state().session.doc.plain_text(wordcraft_doc::StoryRef::Body);
        assert_ne!(document, previous);
        poll.set(true);
        settle_recovery(&mut h);
        assert!(!h.state().ui.backstage, "the requested template must stay visible");
        assert_eq!(h.state().session.document_id(), document);
        assert_eq!(h.state().session.doc.plain_text(wordcraft_doc::StoryRef::Body), text);
        assert_eq!(h.state().session.recovery_documents().len(), 1, "older copies remain available");
        h.state_mut().ui.backstage = true;
        h.run_steps(4);
        h.get_by_label("Recover").click();
        h.run_steps(4);
        assert!(h.state().ui.backstage && h.state().ui.backstage_page == "recover");
        drop(h);
        let _ = std::fs::remove_dir_all(root);
    }
}

#[test]
#[cfg(not(target_arch = "wasm32"))]
fn recovery_does_not_replace_an_explicitly_opened_file() {
    let (_first, root) = recovery_fixture();
    let path = root.join("Requested.txt");
    std::fs::write(&path, "Requested synthetic document").unwrap();
    // The Browse… pick is answered with the file.
    let files = Arc::new(ScriptedFiles::answering([Picked::Paths(vec![path])]));
    let mut app = app_with_files(&files);
    app.ui.language = "en".into();
    app.ui.backstage = true;
    app.ui.backstage_page = "info".into();
    let (mut h, poll) = pending_recovery_harness(app, &root);
    let previous = h.state().session.document_id();
    h.get_by_label("Open").click();
    h.run_steps(4);
    h.get_by_label("Browse…").click();
    h.run_steps(4);
    assert!(!h.state().session.recovery_ready());
    let document = h.state().session.document_id();
    assert_ne!(document, previous);
    poll.set(true);
    settle_recovery(&mut h);
    assert!(!h.state().ui.backstage);
    assert_eq!(h.state().session.document_id(), document);
    assert_eq!(h.state().session.doc.plain_text(wordcraft_doc::StoryRef::Body), "Requested synthetic document");
    assert_eq!(h.state().session.recovery_documents().len(), 1);
    h.state_mut().ui.backstage = true;
    h.run_steps(4);
    h.get_by_label("Recover").click();
    h.run_steps(4);
    assert!(h.state().ui.backstage && h.state().ui.backstage_page == "recover");
    drop(h);
    let _ = std::fs::remove_dir_all(root);
}

#[test]
#[cfg(not(target_arch = "wasm32"))]
fn recovery_page_keeps_dismissed_copies_and_confirms_discard() {
    let (app, root) = recovery_app();
    let mut h = recovery_harness(app);
    settle_recovery(&mut h);
    assert!(h.state().ui.backstage && h.state().ui.backstage_page == "recover");
    h.get_by_label("Continue without restoring").click();
    h.run_steps(4);
    assert!(!h.state().ui.backstage);
    assert_eq!(h.state().session.recovery_documents().len(), 1);
    h.state_mut().run("file.recover", json!({})).unwrap();
    h.run_steps(4);
    h.get_by_label("Discard…").click();
    h.run_steps(4);
    assert!(h.state().dialog_host.open_dialogs().iter().any(|d| d.key.as_deref() == Some("discardRecovery")), "the discard is confirmed first");
    h.get_by_label("Cancel").click();
    h.run_steps(4);
    assert_eq!(h.state().session.recovery_documents().len(), 1);
    h.get_by_label("Discard…").click();
    h.run_steps(4);
    h.get_by_label("Discard").click();
    settle_recovery(&mut h);
    assert!(h.state().session.recovery_documents().is_empty());
    assert!(h.query_by_label("No recovery copies.").is_some());
    drop(h);
    let _ = std::fs::remove_dir_all(root);
}

#[test]
#[cfg(not(target_arch = "wasm32"))]
fn recovery_restore_opens_beside_unsaved_work_without_autosave_permission() {
    let (app, root) = recovery_app();
    let mut h = recovery_harness(app);
    settle_recovery(&mut h);
    h.state_mut().execute("text.insert", json!({"text": "Current unsaved work"})).unwrap();
    let work = h.state().session.document_id();
    h.state_mut().autosave_path = Some(root.join("must-not-write.docx"));
    h.state_mut().run("file.recover", json!({})).unwrap();
    h.run_steps(4);
    assert_eq!(h.state().session.recovery_documents().len(), 1);
    h.get_by_label("Restore").click();
    settle_recovery(&mut h);
    assert!(h.state().dialog_host.open_dialogs().is_empty(), "nothing is replaced, so nothing to ask");
    assert_ne!(h.state().session.document_id(), work);
    assert_eq!(h.state().session.doc.plain_text(wordcraft_doc::StoryRef::Body), "Recovered synthetic work");
    assert!(h.state().session.dirty && h.state().session.path.is_none());
    assert!(h.state().autosave_path.is_none());
    assert!(!h.state().ui.backstage);
    let kept = h.state_mut().session.with_document(work, |s| s.doc.plain_text(wordcraft_doc::StoryRef::Body)).unwrap();
    assert_eq!(kept, "Current unsaved work");
    assert!(h.state().session.recovery_documents().is_empty(), "the restored document keeps the copy");
    assert!(!root.join("must-not-write.docx").exists());
    drop(h);
    let _ = std::fs::remove_dir_all(root);
}

fn personal_dir(tag: &str) -> std::path::PathBuf {
    let d = std::env::temp_dir().join(format!("wc-ui-personal-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn write_template(path: &std::path::Path, text: &str) {
    let mut s = wordcraft_engine::Session::new(wordcraft_doc::Document::new());
    s.run("document.setText", &json!({"text": text})).unwrap();
    wordcraft_engine::io::save_path(path, &s.doc).unwrap();
}

fn new_page_harness(templates: Option<std::path::PathBuf>) -> egui_kittest::Harness<'static, WordApp> {
    let services = Services { templates_dir: templates, ..Default::default() };
    let mut a = WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::new()), services);
    a.run("ui.backstage", json!({"value": true, "page": "new"})).unwrap();
    harness(a, egui::vec2(1440.0, 1100.0))
}

/// Types `text` into New's search field in place of what is there, as a person would: focus the
/// field, select its text and type over it.
fn search_for(h: &mut egui_kittest::Harness<'static, WordApp>, text: &str) {
    h.get_by_label("Search for templates").focus();
    h.run_steps(2);
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
    h.key_press(egui::Key::Backspace);
    h.run_steps(2);
    if !text.is_empty() {
        h.get_by_label("Search for templates").type_text(text);
    }
    h.run_steps(3);
}

/// File › New lists the user templates folder's templates under Personal (#2852), by
/// category; the search covers them; a click makes an untitled document attached to the
/// template.
#[test]
fn new_lists_personal_templates_and_opens_one() {
    let dir = personal_dir("gallery");
    let memo = dir.join("Team memo.dotx");
    write_template(&memo, "Memo from the template");
    std::fs::create_dir_all(dir.join("Legal")).unwrap();
    write_template(&dir.join("Legal").join("Contract.dotx"), "Contract");
    let mut h = new_page_harness(Some(dir.clone()));
    assert!(h.query_by_label("Personal").is_some(), "New has a Personal section");
    assert!(h.query_by_label("Team memo").is_some(), "{:?}", h.state().user_templates_dir());
    assert!(h.query_by_label("Legal").is_some(), "a subfolder is a category");
    assert!(h.query_by_label("Report").is_some(), "the built-in templates stay");

    // The search covers both.
    search_for(&mut h, "memo");
    assert!(h.query_by_label("Team memo").is_some(), "search finds a personal template");
    assert!(h.query_by_label("Contract").is_none() && h.query_by_label("Report").is_none());
    search_for(&mut h, "zzz");
    assert!(h.query_by_label("No templates match your search.").is_some());
    search_for(&mut h, "");

    h.get_by_label("Team memo").click();
    h.run_steps(4);
    let a = h.state();
    assert!(!a.ui.backstage, "File closes");
    assert_eq!(a.session.path, None, "an untitled document");
    assert_eq!(a.session.doc.settings.attached_template.as_deref(), Some(memo.to_string_lossy().as_ref()));
    assert!(a.session.doc.plain_text(wordcraft_doc::StoryRef::Body).contains("Memo from the template"));
    drop(h);
    let _ = std::fs::remove_dir_all(&dir);
}

/// An empty (or not yet made) templates folder says where templates come from; no folder (the
/// web) shows no Personal section. Home keeps only the built-ins.
#[test]
fn personal_section_empty_state_and_no_folder() {
    let dir = personal_dir("empty").join("Templates");
    let h = new_page_harness(Some(dir.clone()));
    assert!(h.query_by_label("Templates you save in your templates folder show up here.").is_some());
    assert!(!dir.exists(), "listing doesn't make the folder");
    drop(h);
    let h = new_page_harness(None);
    assert!(h.query_by_label("Personal").is_none());
}

/// A folder chosen in File › Options › Advanced › File locations replaces the app's own.
#[test]
fn the_options_folder_replaces_the_default() {
    let own = personal_dir("own");
    write_template(&own.join("Own.dotx"), "x");
    let chosen = personal_dir("chosen");
    write_template(&chosen.join("Chosen.dotx"), "y");
    let mut h = new_page_harness(Some(own.clone()));
    assert!(h.query_by_label("Own").is_some());
    h.state_mut().run("file.options", json!({"userTemplatesFolder": chosen.to_string_lossy()})).unwrap();
    h.run_steps(3);
    assert!(h.query_by_label("Chosen").is_some(), "the chosen folder is listed");
    assert!(h.query_by_label("Own").is_none());
    drop(h);
    let _ = std::fs::remove_dir_all(&own);
    let _ = std::fs::remove_dir_all(&chosen);
}

/// A personal template opens a new document like New does: nothing is asked, and the
/// document with unsaved changes stays open beside it.
#[test]
fn a_personal_template_opens_beside_unsaved_work() {
    let dir = personal_dir("dirty");
    let memo = dir.join("Memo.dotx");
    write_template(&memo, "From the template");
    let mut h = new_page_harness(Some(dir.clone()));
    h.state_mut().run("document.setText", json!({"text": "Unsaved work"})).unwrap();
    let first = h.state().session.document_id();
    h.get_by_label("Memo").click();
    h.run_steps(3);
    let body = |h: &egui_kittest::Harness<'static, WordApp>| h.state().session.doc.plain_text(wordcraft_doc::StoryRef::Body);
    assert!(h.state().dialog_host.open_dialogs().is_empty(), "nothing is asked");
    assert!(body(&h).contains("From the template"));
    assert_eq!(h.state().session.doc.settings.attached_template.as_deref(), Some(memo.to_string_lossy().as_ref()));
    h.state_mut().run("view.switchWindows", json!({"id": first})).unwrap();
    assert_eq!(body(&h), "Unsaved work", "the unsaved document is still open");
    assert!(h.state().session.dirty);
    drop(h);
    let _ = std::fs::remove_dir_all(&dir);
}

/// The templates folder is read when New is shown and on Refresh, not on every frame: a file
/// added while New stays open appears only after Refresh, and one added while away appears on the
/// next visit.
#[test]
fn personal_templates_are_listed_once_per_visit_and_on_refresh() {
    let dir = personal_dir("refresh");
    write_template(&dir.join("First.dotx"), "x");
    let mut h = new_page_harness(Some(dir.clone()));
    assert!(h.query_by_label("First").is_some(), "listed when New is shown");
    write_template(&dir.join("Second.dotx"), "y");
    h.run_steps(20);
    assert!(h.query_by_label("Second").is_none(), "not listed again while New stays open");
    h.get_by_label("Refresh").click();
    h.run_steps(3);
    assert!(h.query_by_label("Second").is_some(), "Refresh shows the new template");
    // Leaving New and coming back reads the folder again.
    h.state_mut().run("ui.backstage", json!({"value": true, "page": "open"})).unwrap();
    h.run_steps(2);
    write_template(&dir.join("Third.dotx"), "z");
    assert!(h.query_by_label("Third").is_none(), "New is not shown, so nothing lists it");
    h.state_mut().run("ui.backstage", json!({"value": true, "page": "new"})).unwrap();
    h.run_steps(3);
    assert!(h.query_by_label("Third").is_some(), "a new visit to New lists it");
    drop(h);
    let _ = std::fs::remove_dir_all(&dir);
}

/// A saved templates folder that is a network path, or gone, gives way to the app's own.
#[test]
fn an_unusable_saved_folder_falls_back_to_the_apps_own() {
    let own = personal_dir("fallback");
    let services = Services { templates_dir: Some(own.clone()), ..Default::default() };
    let mut a = WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::new()), services);
    for bad in [r"\\host\share".to_string(), "//host/share".into(), own.join("gone").to_string_lossy().into_owned(), "relative".into()] {
        a.session.prefs.user_templates_folder = Some(bad.clone());
        assert_eq!(a.user_templates_dir(), Some(own.clone()), "{bad}");
    }
    a.session.prefs.user_templates_folder = Some(own.to_string_lossy().into_owned());
    assert_eq!(a.user_templates_dir(), Some(own.clone()));
    let _ = std::fs::remove_dir_all(&own);
}

/// A recent row's Remove (the Delete key on the focused row) and Clear Recent Files (the row's
/// context menu) end in `ui.recentRemove` and `ui.recentClear`: the app's list loses one entry,
/// then all of them, and the page stays open.
#[test]
fn recent_rows_can_be_removed_one_by_one_or_all_at_once() {
    let mut a = new_app();
    a.ui.recent = vec!["/tmp/alpha.docx".into(), "/tmp/beta.docx".into()];
    let mut h = file_page(a, "open");
    assert!(recent_row(&h, "alpha.docx").is_some() && recent_row(&h, "beta.docx").is_some(), "both rows show");
    recent_row(&h, "alpha.docx").expect("alpha's row shows").focus();
    h.run_steps(2);
    h.key_press(egui::Key::Delete);
    h.run_steps(4);
    assert_eq!(h.state().ui.recent, ["/tmp/beta.docx".to_string()], "Remove takes its own row");
    assert!(h.state().ui.backstage, "removing opens nothing");
    assert!(recent_row(&h, "alpha.docx").is_none() && recent_row(&h, "beta.docx").is_some());
    recent_row(&h, "beta.docx").expect("beta's row shows").click_secondary();
    h.run_steps(3);
    h.get_by_label("Clear Recent Files").click();
    h.run_steps(4);
    assert!(h.state().ui.recent.is_empty(), "Clear empties the list");
    assert!(recent_row(&h, "beta.docx").is_none());
}

/// #260, #262: the Info fields once lost typed text when File was left by Escape (or the
/// browser's Save) before the field lost focus, and dropped the document's last editor and
/// title or author. Typing now lands in the document as it happens, one undo step per field,
/// and the browser's Save hands it over with the save stamps advanced.
#[test]
fn info_fields_keep_typed_properties_through_a_web_save() {
    use egui::accesskit::Role;
    // A browser host: the save is a download, answered with the suggested name.
    let files = Arc::new(ScriptedFiles::new().in_browser().saving_suggested());
    let mut a = app_with_files(&files);
    a.session = wordcraft_engine::Session::new(wordcraft_doc::Document::from_text("XYZ"));
    a.session.author = "GAMMA".into();
    let mut h = file_page(a, "info");
    for (field, text) in [("Title", "ALPHA"), ("Author", "BETA")] {
        h.get_by_role_and_label(Role::TextInput, field).click();
        h.run_steps(2);
        for c in text.chars() {
            h.get_by_role_and_label(Role::TextInput, field).type_text(&c.to_string());
            h.run_steps(2);
        }
        assert_eq!(h.get_by_role_and_label(Role::TextInput, field).value().as_deref(), Some(text), "{field} shows what was typed");
    }
    let core = h.state().session.doc.core.clone();
    assert_eq!((core.title.as_str(), core.creator.as_str()), ("ALPHA", "BETA"));
    assert!(h.state().session.dirty, "the property edits are unsaved changes");

    // Save (web: handed to the browser) writes them into docProps/core.xml, with fresh save stamps.
    let a = h.state_mut();
    let created = a.session.doc.core.created.clone();
    a.run("file.save", json!({})).unwrap();
    a.run("file.save", json!({})).unwrap();
    let name = files.requests().last().and_then(|r| r.file_name.clone()).expect("the save asked for a name");
    let bytes = files.file(std::path::Path::new(&name)).expect("the save handed over its bytes");
    let back = wordcraft_engine::io::open_bytes(&name, &bytes).unwrap();
    assert_eq!((back.core.title.as_str(), back.core.creator.as_str()), ("ALPHA", "BETA"));
    assert_eq!(back.core.last_modified_by, "GAMMA");
    assert_eq!(back.core.revision, core.revision + 2, "each save advances the revision");
    assert!(created.is_empty() && !back.core.created.is_empty(), "the first save sets the creation time");
    assert!(!back.core.modified.is_empty());
    assert!(!a.session.dirty);

    // Each field's typing is one undo step.
    a.session.run("edit.undo", &json!({})).unwrap();
    assert_eq!((a.session.doc.core.title.as_str(), a.session.doc.core.creator.as_str()), ("ALPHA", ""));
    a.session.run("edit.undo", &json!({})).unwrap();
    assert_eq!(a.session.doc.core.title, "");
}

/// #3467, #3535: a system dialog that never shows up doesn't leave Save As stuck: Browse in
/// WordCraft… asks the file service for its own chooser (`fallback`), and the path it answers is
/// where the document is saved.
#[test]
#[cfg(all(unix, not(target_os = "macos")))]
fn save_as_saves_in_wordcraft_when_the_system_dialog_never_shows() {
    // A native save writes the picked path itself (the file service only answers where), so the
    // path is one in a folder of our own.
    let dir = std::env::temp_dir().join(format!("wordcraft-save-own-browser-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let saved = dir.join("kept.docx");
    let files = Arc::new(ScriptedFiles::answering([Picked::Paths(vec![saved.clone()])]));
    let mut a = app_with_files(&files);
    a.run("text.insert", json!({"text": "Saved past a silent portal"})).unwrap();
    let mut h = file_page(a, "saveAs");
    assert!(files.requests().is_empty(), "nothing is asked until a button is pressed");
    h.get_by_label("Browse in WordCraft…").click();
    h.run_steps(6);
    let asked = files.requests();
    assert_eq!(asked.len(), 1, "one pick: {asked:?}");
    assert!(asked[0].fallback, "the request asks for WordCraft's own chooser");
    assert_eq!(asked[0].kind, craft::host::PickKind::Save);
    assert_eq!(h.state().session.path.as_deref(), Some(saved.as_path()));
    assert!(!h.state().ui.backstage, "the page closes once saved");
    let bytes = std::fs::read(&saved).expect("the picked path holds the saved document");
    let _ = std::fs::remove_dir_all(&dir);
    let back = wordcraft_engine::io::open_bytes("kept.docx", &bytes).unwrap();
    assert!(back.plain_text(wordcraft_doc::StoryRef::Body).contains("Saved past a silent portal"));
}

/// #3535: File › Export offers WordCraft's own chooser as well, for its formats: the request
/// asks for the fallback with the PDF type.
#[test]
#[cfg(all(unix, not(target_os = "macos")))]
fn export_page_offers_wordcrafts_own_browser() {
    let files = Arc::new(ScriptedFiles::new());
    let mut h = file_page(app_with_files(&files), "export");
    h.get_by_label("Browse in WordCraft…").click();
    h.run_steps(4);
    let asked = files.requests();
    let request = asked.last().expect("the button asked the file service");
    assert!(request.fallback, "the request asks for WordCraft's own chooser: {request:?}");
    assert!(request.filters.iter().any(|f| f.extensions.iter().any(|e| e == "pdf")), "with the PDF type: {request:?}");
}

/// The side menu's rows go where the page says: Save on an unnamed document opens Save As and no
/// file dialog; Save on a named one saves and closes File; Close on a clean document closes it at
/// once and on an edited one asks first (#1124, #1712); Save As opens its page.
#[test]
fn backstage_save_navigates_to_save_as_when_unnamed() {
    // Save, unnamed.
    let files = Arc::new(ScriptedFiles::new());
    let mut h = file_page(app_with_files(&files), "home");
    h.get_by_label("Save").click();
    h.run_steps(4);
    assert_eq!(h.state().ui.backstage_page, "saveAs", "unnamed Save opens Save As page");
    assert!(h.state().ui.backstage, "backstage remains open");
    assert!(files.requests().is_empty(), "no file dialog");

    // Save, named.
    let dir = std::env::temp_dir().join(format!("wordcraft-backstage-save-named-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("named.docx");
    let mut a = new_app();
    a.run("text.insert", json!({"text": "Named"})).unwrap();
    a.run("file.saveAs", json!({"path": path.to_string_lossy()})).unwrap();
    a.run("text.insert", json!({"text": " and edited"})).unwrap();
    let mut h = file_page(a, "home");
    h.get_by_label("Save").click();
    h.run_steps(4);
    assert!(!h.state().ui.backstage, "a successful save closes File");
    assert!(!h.state().session.dirty, "and the changes are saved");
    let _ = std::fs::remove_dir_all(&dir);

    // Close, clean.
    let mut h = file_page(new_app(), "home");
    h.get_by_label("Close").click();
    h.run_steps(1);
    assert!(h.state().closing.is_none(), "no prompt");
    assert!(!h.state().has_document() && !h.state().quit_requested);

    // Close, edited: asks; Cancel keeps the document and the page, Don't Save closes the document.
    let mut a = new_app();
    a.run("text.insert", json!({"text": "Draft"})).unwrap();
    let mut h = file_page(a, "info");
    let export = h.get_by_label("Export").rect().min.y;
    assert!(h.get_by_label("Close").rect().min.y > export, "Close comes after Export");
    h.get_by_label("Close").click();
    h.run_steps(4);
    assert!(h.state().closing.is_some(), "unsaved changes ask first");
    assert!(h.state().has_document());
    h.state_mut().run("ui.saveChanges", json!({"answer": "cancel"})).unwrap();
    h.run_steps(4);
    assert!(h.state().has_document(), "Cancel keeps the document open");
    assert!(h.state().ui.backstage && h.state().ui.backstage_page == "info", "Cancel leaves the Backstage as it was");
    h.get_by_label("Close").click();
    h.run_steps(4);
    h.state_mut().run("ui.saveChanges", json!({"answer": "dontSave"})).unwrap();
    assert!(!h.state().has_document() && !h.state().quit_requested, "Don't Save closes the document, not the window (#1712)");

    // Save As.
    let mut h = file_page(new_app(), "home");
    h.get_by_label("Save As").click();
    h.run_steps(4);
    assert_eq!(h.state().ui.backstage_page, "saveAs");
}

/// Repro: a brand-new document, typed text, File > Save (unnamed) > Save As page > Browse…,
/// answered by the file service, should land back on the document, not the "no document" screen.
#[test]
fn backstage_save_browse_native_dialog_lands_on_the_document() {
    // A native save writes the picked path itself, so the path is one in a folder of our own.
    let dir = std::env::temp_dir().join(format!("wordcraft-save-browse-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("Document1.docx");
    let files = Arc::new(ScriptedFiles::answering([Picked::Paths(vec![path.clone()])]));
    let mut a = app_with_files(&files);
    a.run("text.insert", json!({"text": "Hello from the repro"})).unwrap();
    let mut h = file_page(a, "home");
    h.get_by_label("Save").click();
    h.run_steps(4);
    assert_eq!(h.state().ui.backstage_page, "saveAs", "unnamed Save opens Save As page");
    h.get_by_label("Browse…").click();
    h.run_steps(6);
    assert!(h.state().session.path.as_deref() == Some(path.as_path()), "the document was saved to the picked path");
    assert!(path.is_file(), "the picked path holds the document");
    let _ = std::fs::remove_dir_all(&dir);
    assert!(!h.state().ui.backstage, "backstage closes once saved");
    assert!(h.state().has_document(), "the document stays open after Save");
}

/// The Save As page's own render loop skips the status bar while Backstage is open, so a
/// write failure (no permission, disk full, a bad path) needs its own place to say why, or
/// the page just sits there with a Browse… button and no clue.
#[test]
fn backstage_save_browse_write_failure_shows_on_the_save_as_page() {
    let files = Arc::new(ScriptedFiles::answering([Picked::Failed("the disk is full".into())]));
    let mut a = app_with_files(&files);
    a.run("text.insert", json!({"text": "unsaved text"})).unwrap();
    let mut h = file_page(a, "home");
    h.get_by_label("Save").click();
    h.run_steps(4);
    h.get_by_label("Browse…").click();
    h.run_steps(6);
    assert!(h.state().ui.backstage && h.state().ui.backstage_page == "saveAs", "nothing was actually saved");
    let msg = h.state().status.as_ref().map(|s| s.text.clone()).unwrap_or_default();
    assert!(!msg.is_empty(), "the failure set a status message");
    // The same text is rendered on the Save As page itself, not just the (invisible, while
    // Backstage is open) status bar.
    h.get_by_label(msg.as_str());
}

/// Info shows each property's name beside its field, the names in one column at the fields' start, as production's
/// table did; the port once named the fields for assistive technology only, so the page showed five bare boxes.
#[test]
fn info_properties_show_their_names_beside_the_fields() {
    use egui::accesskit::Role;
    let mut a = new_app();
    a.run("text.insert", json!({"text": "Some text"})).unwrap();
    let h = file_page(a, "info");
    // The framework paints a `Field`'s side label as part of the field (the label step names the control by it and
    // claims the label's input), so the label has no node of its own to find: the room it takes beside the field
    // is what shows. The fields start well right of the column's start, where the full-width button below them
    // begins, and each is named by its label for assistive technology.
    let column = h.get_by_label("Advanced Properties…").rect().min.x;
    let mut fields = Vec::new();
    for name in ["Title", "Subject", "Author", "Keywords", "Category"] {
        let field = h.get_by_role_and_label(Role::TextInput, name).rect();
        assert!(field.min.x >= column + 40.0, "{name}'s label has room before its field: column {column}, field {field:?}");
        fields.push(field);
    }
    let first = fields.first().map(|f| f.min.x).unwrap_or_default();
    assert!(fields.iter().all(|f| (f.min.x - first).abs() < 1.0), "the fields line up: {fields:?}");
}

/// The Print page's preview takes the room beside the settings, as production's: the port once showed it as a
/// thumbnail in a content-sized row at the top of the page.
#[test]
fn print_preview_fills_the_page_beside_the_settings() {
    let mut a = new_app();
    a.run("text.insert", json!({"text": "Some text"})).unwrap();
    let mut h = file_page(a, "print");
    h.run_steps(6);
    let pane = h.query_all_by_label("Sheet 1 of 1").map(|node| node.rect()).max_by(|a, b| a.height().total_cmp(&b.height()));
    let pane = pane.expect("the preview shows the first sheet");
    assert!(pane.height() > 500.0, "the preview takes the page's height: {pane:?}");
    let pdf = h.get_by_label("PDF document (*.pdf)").rect();
    assert!(pane.min.x > pdf.max.x, "the preview is beside the settings: {pane:?} {pdf:?}");
    // Production's settings column is 380 pt (a dialog's least content width); the column shares the name "Print" with
    // the side menu's row and the Print heading, and is the tallest of them.
    let column = h.query_all_by_label("Print").map(|node| node.rect()).max_by(|a, b| a.height().total_cmp(&b.height())).expect("the settings column");
    assert!((370.0..=420.0).contains(&column.width()), "the settings column is production's 380 pt: {column:?}");
}

/// #6639: when the whole side menu fits, About sits in its footer, below the last page row.
#[test]
fn about_stays_at_the_bottom_of_a_side_menu_that_fits() {
    let h = file_page(new_app(), "home");
    let about = h.get_by_label("About").rect();
    let options = h.get_by_label("Options").rect();
    assert!(about.min.y >= options.max.y, "About is below Options: {about:?} {options:?}");
    assert!(about.max.y <= 900.0, "About is inside the window: {about:?}");
    // The menu is as tall as the window, so its footer is at the window's foot, not under Options.
    assert!(about.max.y > 820.0, "About is at the foot of the window: {about:?}");
    let discord = h.get_by_label("Discord community").rect();
    assert!(discord.max.y <= about.min.y + 1.0, "the community link is above About: {discord:?} {about:?}");
    assert!((about.min.x - options.min.x).abs() < 1.0 && about.width() > 80.0, "About spans the menu: {about:?}");
    assert!((about.min.x - options.min.x).abs() < 1.0, "in the same column: {about:?} {options:?}");
}

/// The side menu is production's rail: 200 pt wide (five rail columns, `LeastWidth::SideMenu`), its page rows about
/// 38 pt tall one after another. The footer's links are rows of the same place, so they read in the rail's white ink;
/// production's were a little shorter (32 pt), which the side menu place has no row for.
#[test]
fn the_side_menu_rows_are_production_s_rail_rows() {
    let h = file_page(new_app(), "home");
    let (info, recover) = (h.get_by_label("Info").rect(), h.get_by_label("Recover").rect());
    let about = h.get_by_label("About").rect();
    // The least width holds in the row beside the page: before, the row gave the menu only its widest name (~123 pt).
    assert!((190.0..=260.0).contains(&info.width()), "the menu is production's 200 pt rail: {info:?}");
    assert!((34.0..=44.0).contains(&info.height()), "a page row is a rail row: {info:?}");
    assert!((recover.min.y - info.max.y).abs() < 1.0, "the rows follow each other without a gap: {info:?} {recover:?}");
    assert!(about.height() <= info.height() + 1.0, "the footer link is no taller than a page row: {info:?} {about:?}");
}

/// A refused Word XML export (the typed name's `.xml` twin already exists) is said on the Export
/// page itself: Backstage hides the status bar, so the person would otherwise see nothing.
#[test]
fn export_refusal_shows_on_the_export_page() {
    let dir = std::env::temp_dir().join(format!("wordcraft-backstage-export-error-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("letter.xml"), "taken").unwrap();
    let bare = dir.join("letter").to_string_lossy().into_owned();
    let mut a = new_app();
    a.run("text.insert", json!({"text": "Dear reader"})).unwrap();
    let mut h = file_page(a, "export");
    let err = h.state_mut().finish_pick(crate::file_dialogs::AfterPick::Export { ext: "xml".into() }, Some(bare)).unwrap_err();
    h.run_steps(4);
    assert!(h.state().ui.backstage, "still on the Export page");
    assert!(err.contains("letter.xml already exists"), "{err}");
    h.get_by_label(err.as_str());
    assert_eq!(std::fs::read_to_string(dir.join("letter.xml")).unwrap(), "taken", "nothing replaced");
    let _ = std::fs::remove_dir_all(&dir);
}

/// A refused Word XML export (the typed name's `.xml` twin already exists) is said on the Print
/// page itself: Backstage hides the status bar, so the person would otherwise see nothing.
#[test]
fn export_refusal_shows_on_the_print_page() {
    let dir = std::env::temp_dir().join(format!("wordcraft-backstage-print-export-error-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("letter.xml"), "taken").unwrap();
    let bare = dir.join("letter").to_string_lossy().into_owned();
    let mut a = new_app();
    a.run("text.insert", json!({"text": "Dear reader"})).unwrap();
    let mut h = file_page(a, "print");
    let err = h.state_mut().finish_pick(crate::file_dialogs::AfterPick::Export { ext: "xml".into() }, Some(bare)).unwrap_err();
    h.run_steps(4);
    assert!(h.state().ui.backstage, "still on the Print page");
    assert!(err.contains("letter.xml already exists"), "{err}");
    h.get_by_label(err.as_str());
    assert_eq!(std::fs::read_to_string(dir.join("letter.xml")).unwrap(), "taken", "nothing replaced");
    let _ = std::fs::remove_dir_all(&dir);
}

/// A cancelled export dialog drops an earlier refusal: it is not news any more.
#[test]
fn cancelling_an_export_clears_its_page_error() {
    let mut a = new_app();
    a.export_error = "old refusal".into();
    a.finish_pick(crate::file_dialogs::AfterPick::Export { ext: "xml".into() }, None).unwrap();
    assert!(a.export_error.is_empty());
}

/// The export error belongs to the window that refused the export: another window running a
/// frame outside Backstage must not erase it while this one still shows the Export page.
#[test]
fn export_error_belongs_to_its_window() {
    let dir = std::env::temp_dir().join(format!("wordcraft-backstage-export-two-windows-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("letter.xml"), "taken").unwrap();
    let bare = dir.join("letter").to_string_lossy().into_owned();
    let mut a = new_app();
    a.run("text.insert", json!({"text": "Dear reader"})).unwrap();
    a.run("ui.backstage", json!({"value": true, "page": "export"})).unwrap();
    // Window A (this one) is refused; window B is another window's state, outside Backstage.
    let mut b = crate::window_ui::WindowUi::new(a.session.document_id());
    let err = a.finish_pick(crate::file_dialogs::AfterPick::Export { ext: "xml".into() }, Some(bare)).unwrap_err();
    assert_eq!(a.export_error, err);
    // B's frame: swap its state in, run frames outside Backstage, swap back.
    a.swap_window(&mut b);
    assert!(!a.ui.backstage && a.export_error.is_empty(), "B has its own, empty, error");
    let mut h = harness(a, egui::vec2(1440.0, 900.0));
    h.state_mut().swap_window(&mut b);
    assert!(h.state().ui.backstage, "A is still on the Export page");
    assert_eq!(h.state().export_error, err, "B's frames left A's message alone");
    let _ = std::fs::remove_dir_all(&dir);
}

/// A File page's title is production's 26 pt, well above its section headings: the display role WordCraft's sizes
/// give it, not the suite's 38 pt brand title.
#[test]
fn a_file_pages_title_is_productions_size() {
    use craft::theme::{Appearance, Contrast, Mode, Size, TextSize, TypeRole};
    let sizes = new_app().root_env().sizes;
    let themes = craft::theme::builtins();
    let theme = themes.iter().find(|theme| theme.id.as_str() == craft::theme::SUITE_DEFAULT).unwrap();
    for size in [Size::Compact, Size::Comfortable] {
        let appearance = Appearance { mode: Mode::Light, contrast: Contrast::Standard, size, text_size: TextSize::default(), reduced_motion: false };
        let type_ = craft::theme::resolve_sized(theme, appearance, None, &sizes).tokens.typography;
        let (title, section) = (type_.get(TypeRole::Display), type_.get(TypeRole::Heading));
        assert!((title.size - 26.0).abs() < 0.5, "the page title is 26 pt in {size:?}: {title:?}");
        assert!(title.size > section.size + 6.0, "the title stands above a section heading: {title:?} over {section:?}");
    }
}
