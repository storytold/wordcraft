use serde_json::json;

use super::Origin;
use crate::WordApp;
use crate::services::Services;

fn app() -> WordApp {
    WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::new()), Services::default())
}

#[test]
fn without_the_desktop_hook_nothing_starts() {
    let mut a = app();
    for (id, params) in [
        ("ui.drive.start", json!({})),
        ("ui.drive.signIn", json!({})),
        ("ui.drive.list", json!({"search": "x"})),
        ("ui.drive.open", json!({"id": "abc"})),
        ("ui.drive.save", json!({})),
        ("ui.drive.reload", json!({})),
    ] {
        let r = a.run(id, params);
        assert!(r.is_err(), "{id} should be unavailable without a profile: {r:?}");
    }
    #[cfg(not(target_arch = "wasm32"))]
    assert!(a.drive.worker.is_none(), "no worker thread, so no network or keychain in tests");
    let s = a.run("ui.drive.status", json!({})).unwrap();
    assert_eq!(s["available"], json!(false));
}

#[test]
fn hostile_params_never_panic() {
    let mut a = app();
    let junk = [json!(null), json!(5), json!("x"), json!([1, 2]), json!({"id": 7, "search": {"a": 1}, "name": [1], "copy": "yes", "more": 3})];
    for name in ["status", "start", "reload", "signIn", "cancel", "signOut", "list", "open", "save", "nope", ""] {
        for p in &junk {
            let _ = a.run(&format!("ui.drive.{name}"), p.clone());
        }
    }
}

#[test]
fn file_save_goes_to_drive_only_for_drive_documents() {
    let mut a = app();
    // An ordinary document: not intercepted, so the usual save (and its Save As prompt) runs.
    assert!(super::intercept_save(&mut a, "file.save", &json!({})).is_none());
    let doc = a.session.document_id();
    a.drive.origins.insert(doc, Origin { id: "f1".into(), name: "Essay.docx".into(), version: "3".into(), google_doc: false });
    // A Drive document: Save is Drive's (refused here only because tests have no Drive).
    let r = super::intercept_save(&mut a, "file.save", &json!({}));
    assert!(matches!(r, Some(Err(_))), "{r:?}");
    // An explicit path is a local save, and Save As is never intercepted.
    assert!(super::intercept_save(&mut a, "file.save", &json!({"path": "/tmp/x.docx"})).is_none());
    assert!(super::intercept_save(&mut a, "file.saveAs", &json!({})).is_none());
    // Nothing marked the document saved.
    assert!(a.session.path.is_none());
}

#[test]
fn save_names() {
    let mut a = app();
    let doc = a.session.document_id();
    assert!(super::save_name(&a, Some("Report")).ends_with("Report.docx"));
    assert_eq!(super::save_name(&a, Some("Report.odt")), "Report.odt");
    assert_eq!(super::save_name(&a, Some("Report.docx")), "Report.docx");
    a.drive.origins.insert(doc, Origin { id: "f1".into(), name: "Essay.docx".into(), version: "3".into(), google_doc: false });
    assert_eq!(super::save_name(&a, None), "Essay.docx");
    // A Google Doc's copy is named after the document, as .docx, never the Doc itself.
    a.drive.origins.insert(doc, Origin { id: "g1".into(), name: "Notes".into(), version: "3".into(), google_doc: true });
    a.session.file_name = Some("Notes.docx".into());
    assert_eq!(super::save_name(&a, None), "Notes.docx");
}

#[test]
fn drive_page_draws_without_a_profile() {
    let mut a = app();
    let ctx = egui::Context::default();
    let mut out = ctx.run_ui(egui::RawInput::default(), |ui| super::page(&mut a, ui));
    // The font textures aren't uploaded anywhere here.
    out.textures_delta.clear();
    assert!(!out.shapes.is_empty());
}
