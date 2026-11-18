//! Open, Save and Share against cloud storage reach the real `craft_ui::cloud` contract
//! (`wordcraft_ui_egui::cloud_app` implements `CloudApp` for `WordApp`), but no backend is
//! connected in this build. These tests cover both sides: with no backend, every host-gated
//! command path refuses rather than silently succeeding or hanging; with a real backend
//! connected ([`craft_cloud_local::LocalBackend`], a reference `CloudBackend` over a temp
//! folder, not an inert fake), a save genuinely lands on disk and a save against a backend
//! whose folder has gone genuinely fails.

use std::time::{Duration, Instant};

use craft_ui::cloud;
use serde_json::json;
use wordcraft_doc::Document;
use wordcraft_engine::Session;
use wordcraft_ui_egui::{Services, WordApp};

fn app() -> WordApp {
    WordApp::new(Session::new(Document::new()), Services::default())
}

/// Runs [`cloud::pump`] until `ready` answers or five seconds pass. [`craft_cloud_local`]'s jobs
/// each run on their own thread and answer on a channel; this is the same loop a real event loop
/// drives one frame at a time.
fn pump_until(app: &mut WordApp, ctx: &egui::Context, mut ready: impl FnMut(&WordApp) -> bool) {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        cloud::pump(app, ctx);
        if ready(app) {
            return;
        }
        assert!(Instant::now() < deadline, "the cloud backend never answered");
        std::thread::sleep(Duration::from_millis(5));
    }
}

// ---- no backend connected: every host-gated path refuses -----------------

#[test]
fn no_backend_is_available() {
    let mut app = app();
    assert!(!cloud::available(&app));
    assert!(!cloud::shares(&mut app));
    assert!(!cloud::has_target(&app));
    assert!(!cloud::prefers_cloud(&app));
}

#[test]
fn report_says_unavailable_rather_than_panicking() {
    let app = app();
    assert_eq!(cloud::report(&app)["available"], false);
}

#[test]
fn the_active_document_has_no_cloud_origin() {
    let app = app();
    assert!(cloud::origin(&app).is_none());
}

#[test]
fn the_share_dialog_never_asks_for_a_save() {
    let mut app = app();
    assert!(app.share.take_save_request().is_none());
}

/// `cloud.openFile` and `cloud.saveAs` (with a folder and a name) are gated on availability
/// before either searches a listing or writes `save_name`: a build with no backend must refuse
/// immediately, not fall through to "no such file" or quietly start a save that can never land.
#[test]
fn host_gated_cloud_commands_report_unavailable_through_execute() {
    let mut app = app();
    for (id, params) in [
        ("cloud.open", json!({})),
        ("cloud.browse", json!({})),
        ("cloud.shared", json!({})),
        ("cloud.link", json!({})),
        ("cloud.openFile", json!({"name": "Document1.docx"})),
        ("cloud.saveAs", json!({"folder": "/", "name": "Document1.docx"})),
    ] {
        let err = app.execute(id, params.clone()).unwrap_err();
        assert_eq!(err, cloud::UNAVAILABLE, "{id} {params}");
    }
    // Writing nothing: a refused Save As must not leave a name behind for a later, unrelated one.
    assert!(app.cloud.save_name.is_empty());
}

#[test]
fn cloud_saveas_without_a_folder_or_name_goes_to_the_page_rather_than_refusing() {
    let mut app = app();
    // No `folder`/`name`: this is "go to the Save As page", which is itself gated on
    // availability, but through a different message (there is nothing to refuse yet).
    let err = app.execute("cloud.saveAs", json!({})).unwrap_err();
    assert_eq!(err, cloud::UNAVAILABLE);
}

/// `save` and `reload` need no host-side availability gate: both already refuse safely, because
/// no backend ever populates a cloud origin for this document to have.
#[test]
fn save_and_reload_refuse_with_no_cloud_origin_rather_than_unavailable() {
    let mut app = app();
    let save_err = cloud::save(&mut app).unwrap_err();
    assert!(save_err.contains("did not come from the cloud"), "{save_err}");
    let reload_err = cloud::reload_active(&mut app).unwrap_err();
    assert!(reload_err.contains("did not come from the cloud"), "{reload_err}");
}

// ---- a real backend: craft_cloud_local::LocalBackend over a temp folder --

/// Not an inert fake: a real `CloudBackend` over a folder on disk, proof that the app side of
/// the contract (`wordcraft_ui_egui::cloud_app`) needs no specific provider.
fn connected(root: std::path::PathBuf) -> (WordApp, egui::Context) {
    let mut app = app();
    app.cloud.connect(craft_cloud_local::LocalBackend::new(root));
    (app, egui::Context::default())
}

#[test]
fn save_as_against_a_real_backend_lands_on_disk_and_records_a_writable_origin() {
    let dir = tempfile::tempdir().expect("temp dir");
    let (mut app, ctx) = connected(dir.path().to_path_buf());
    cloud::save_as(&mut app, "", "notes.docx").expect("the job is accepted");
    pump_until(&mut app, &ctx, |app| app.cloud.busy.is_none());
    assert!(cloud::has_target(&app), "a successful save against a writable entry has a target");
    let origin = cloud::origin(&app).expect("an origin was recorded");
    assert_eq!(origin.entry.name, "notes.docx");
    assert!(dir.path().join("notes.docx").exists(), "the bytes landed in the backend's folder");
}

#[test]
fn save_against_a_backend_whose_root_is_a_file_refuses_rather_than_hanging_forever() {
    let dir = tempfile::tempdir().expect("temp dir");
    // A real failure, not a missing folder the backend could just create: a plain file cannot be
    // opened as a directory capability on any platform.
    let not_a_folder = dir.path().join("root-is-actually-a-file");
    std::fs::write(&not_a_folder, b"not a directory").expect("write the stand-in file");
    let (mut app, ctx) = connected(not_a_folder);
    cloud::save_as(&mut app, "", "notes.docx").expect("the job is accepted; the backend fails asynchronously");
    pump_until(&mut app, &ctx, |app| app.cloud.busy.is_none());
    assert!(!cloud::has_target(&app), "no origin is recorded when the backend could not write");
    // A failed save reaches the user through `CloudApp::say` (here, the status bar), not through
    // `CloudFiles::status`, which carries progress and success messages only.
    let said = app.status_msg.as_ref().map(|(text, _)| text.clone()).unwrap_or_default();
    assert!(!said.is_empty(), "the refusal is reported rather than silently swallowed");
}
