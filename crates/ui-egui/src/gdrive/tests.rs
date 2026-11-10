use serde_json::json;

use super::Origin;
use crate::WordApp;
use crate::services::Services;

fn app_off() -> WordApp {
    WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::new()), Services::default())
}

/// An app with Google Drive turned on (File › Options › Accounts).
fn app() -> WordApp {
    let mut a = app_off();
    a.run("ui.drive.enable", json!({"value": true})).unwrap();
    a
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

#[test]
fn title_says_saved_to_drive_only_for_drive_files() {
    let mut a = app();
    let doc = a.session.document_id();
    assert!(!super::saved_in_drive(&a));
    a.drive.origins.insert(doc, Origin { id: "g1".into(), name: "Notes".into(), version: "1".into(), google_doc: true });
    assert!(!super::saved_in_drive(&a), "a Google Doc's Word copy isn't saved anywhere yet");
    a.drive.origins.insert(doc, Origin { id: "f1".into(), name: "Essay.docx".into(), version: "1".into(), google_doc: false });
    assert!(super::saved_in_drive(&a));
}

#[test]
fn drive_pages_commands_work_with_no_document_but_save_needs_one() {
    for id in [
        "ui.drive.status",
        "ui.drive.start",
        "ui.drive.reload",
        "ui.drive.signIn",
        "ui.drive.cancel",
        "ui.drive.signOut",
        "ui.drive.list",
        "ui.drive.open",
        "ui.drive.enable",
        "ui.drive.client",
    ] {
        assert!(crate::works_without_document(id), "{id} works with no document");
    }
    assert!(!crate::works_without_document("ui.drive.save"), "saving needs a document");
}

/// Save follows the last destination: a local Save As makes Save local; saving to Drive again
/// makes Save go back to Drive.
#[cfg(not(target_arch = "wasm32"))]
#[test]
fn save_follows_the_last_destination() {
    let mut a = app();
    let doc = a.session.document_id();
    a.drive.origins.insert(doc, Origin { id: "f1".into(), name: "Essay.docx".into(), version: "3".into(), google_doc: false });
    assert!(super::intercept_save(&mut a, "file.save", &json!({})).is_some(), "opened from Drive: Save goes to Drive");
    assert!(super::saved_in_drive(&a));
    // Save As to a local file: the document now has a path, so Save writes it.
    a.session.path = Some(std::env::temp_dir().join("Essay.docx"));
    assert!(super::intercept_save(&mut a, "file.save", &json!({})).is_none(), "after a local Save As, Save is local");
    assert!(!super::saved_in_drive(&a));
    assert!(a.run("ui.drive.status", json!({})).unwrap()["document"].is_null());
    // Saving that local document to Drive makes Drive its home again.
    let file = wordcraft_gdrive::DriveFile {
        id: "f2".into(),
        name: "Essay.docx".into(),
        mime_type: wordcraft_gdrive::drive::MIME_DOCX.into(),
        modified_time: String::new(),
        version: "1".into(),
        size: Some(10),
    };
    let rev = a.session.rev();
    let local = a.drive.local_saves.get(&doc).copied().unwrap_or(0);
    super::native::apply(&mut a, super::native::Event::Saved { doc, rev, local, file: file.clone(), marked_saved: true });
    assert!(a.session.path.is_none());
    assert_eq!(a.drive.origin(doc).map(|o| o.id.as_str()), Some("f2"));
    assert!(super::intercept_save(&mut a, "file.save", &json!({})).is_some(), "saved to Drive: Save goes to Drive");
}

#[cfg(not(target_arch = "wasm32"))]
fn drive_file() -> wordcraft_gdrive::DriveFile {
    wordcraft_gdrive::DriveFile {
        id: "f2".into(),
        name: "Essay.docx".into(),
        mime_type: wordcraft_gdrive::drive::MIME_DOCX.into(),
        modified_time: String::new(),
        version: "1".into(),
        size: Some(10),
    }
}

/// Runs `during` between a Drive upload starting and ending, then ends the upload.
#[cfg(not(target_arch = "wasm32"))]
fn upload_around(a: &mut WordApp, during: impl FnOnce(&mut WordApp)) {
    let doc = a.session.document_id();
    let local = a.drive.local_saves.get(&doc).copied().unwrap_or(0);
    a.session.stamp_save();
    during(a);
    let rev = a.session.rev();
    super::native::apply(a, super::native::Event::Saved { doc, rev, local, file: drive_file(), marked_saved: true });
}

/// A local save that set the document's home while a Drive upload was going is newer: the upload
/// ending doesn't take the destination back to Drive. A plain-text copy elsewhere isn't such a
/// save, and a revision stamp already at its maximum doesn't hide one.
#[cfg(not(target_arch = "wasm32"))]
#[test]
fn a_newer_local_save_keeps_its_destination() {
    let dir = std::env::temp_dir().join(format!("wordcraft-gdrive-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let docx = dir.join("Essay.docx");
    let path = |p: &std::path::Path| json!({"path": p.to_string_lossy()});

    // A Drive document saved locally (Save As) during the upload.
    let mut a = app();
    upload_around(&mut a, |a| {
        a.run("file.saveAs", path(&docx)).unwrap();
    });
    assert_eq!(a.session.path.as_deref(), Some(docx.as_path()), "Save As during the upload keeps its path");
    assert!(super::intercept_save(&mut a, "file.save", &json!({})).is_none(), "Save stays local");

    // Saved again to the same local file, with the revision stamp already at its maximum.
    a.session.doc.core.revision = u32::MAX;
    upload_around(&mut a, |a| {
        a.run("file.save", path(&docx)).unwrap();
    });
    assert_eq!(a.session.path.as_deref(), Some(docx.as_path()), "a save to the same file is newer too");

    // A plain-text copy to another file doesn't make that file the document's home: Drive wins.
    upload_around(&mut a, |a| {
        a.run("file.saveAs", path(&dir.join("Essay.txt"))).unwrap();
        assert_eq!(a.session.path.as_deref(), Some(docx.as_path()), "the .txt is a copy");
    });
    assert!(a.session.path.is_none(), "the upload makes Drive the home");
    assert!(super::intercept_save(&mut a, "file.save", &json!({})).is_some(), "Save goes to Drive");
    let _ = std::fs::remove_dir_all(&dir);
}

/// Off until turned on in File › Options › Accounts: the commands say so, Save is an ordinary
/// save, and turning it off leaves File › Google Drive.
#[test]
fn google_drive_is_off_until_turned_on() {
    let mut a = app_off();
    assert_eq!(a.run("ui.drive.status", json!({})).unwrap()["enabled"], json!(false));
    assert_eq!(a.run("ui.drive.enable", json!({})).unwrap(), json!({"enabled": false}), "no value reads it");
    let r = a.run("ui.drive.list", json!({})).unwrap_err();
    assert!(r.contains("Options › Accounts"), "{r}");
    let doc = a.session.document_id();
    a.drive.origins.insert(doc, Origin { id: "f1".into(), name: "Essay.docx".into(), version: "3".into(), google_doc: false });
    assert!(super::intercept_save(&mut a, "file.save", &json!({})).is_none(), "off: Save isn't Drive's");
    assert!(a.run("ui.drive.enable", json!({"value": "yes"})).is_err());

    a.run("ui.drive.enable", json!({"value": true})).unwrap();
    assert!(a.ui.google_drive);
    assert!(super::intercept_save(&mut a, "file.save", &json!({})).is_some(), "on: Save goes to Drive");
    a.ui.backstage_page = "drive".into();
    a.run("ui.drive.enable", json!({"value": false})).unwrap();
    assert_eq!(a.ui.backstage_page, "home", "turning it off leaves the page");
    // The setting is kept between runs in ui.json.
    let saved = serde_json::to_value(&a.ui).unwrap();
    assert_eq!(saved["googleDrive"], json!(false));
}

/// File › Options › Accounts saves the Google client: refused without the desktop hook, and a
/// non-string ID never panics.
#[test]
fn the_accounts_client_needs_the_desktop_app() {
    let mut a = app();
    assert!(a.run("ui.drive.client", json!({"clientId": "123-abc.apps.googleusercontent.com"})).is_err());
    assert!(a.run("ui.drive.client", json!({"clientId": 5})).is_err());
    assert!(a.run("file.options", json!({"pane": "accounts"})).is_ok(), "the Accounts pane exists");
}

/// Every format File › Export offers for Google Drive is written from bytes, so the copy can be
/// uploaded; the command refuses others and needs the desktop app.
#[test]
fn drive_export_formats_all_serialize() {
    let mut a = app();
    a.session = wordcraft_engine::Session::new(wordcraft_doc::Document::from_text("Synthetic export text"));
    let opts = wordcraft_engine::cmd::file::export_options(&a.session, &json!({}));
    for ext in super::EXPORT_FORMATS {
        let bytes = wordcraft_engine::io::save_bytes_with(&format!("Copy.{ext}"), &a.session.doc, None, &opts);
        assert!(bytes.is_ok_and(|b| !b.is_empty()), "{ext}");
    }
    assert!(a.run("ui.drive.export", json!({"format": "png"})).unwrap_err().contains("one of"));
    assert!(a.run("ui.drive.export", json!({"format": 3})).is_err());
    assert!(a.run("ui.drive.export", json!({"format": "pdf"})).unwrap_err().contains("desktop app"));
    assert!(!super::exports_to_drive(&a), "not signed in: Export asks for a folder");
}

/// A Drive export is a copy: the document keeps its home and its unsaved changes.
#[cfg(not(target_arch = "wasm32"))]
#[test]
fn a_drive_export_changes_nothing_about_the_document() {
    let mut a = app();
    let local = std::env::temp_dir().join("Essay.docx");
    a.session.path = Some(local.clone());
    a.session.dirty = true;
    let mut file = drive_file();
    file.name = "Essay.pdf".into();
    super::native::apply(&mut a, super::native::Event::Exported(file));
    assert_eq!(a.session.path.as_deref(), Some(local.as_path()));
    assert!(a.session.dirty);
    assert!(a.drive.origin(a.session.document_id()).is_none());
}

#[test]
fn drive_exports_drop_the_documents_extension() {
    let mut a = app();
    a.session.file_name = Some("Essay.docx".into());
    assert_eq!(super::export_stem(&a), "Essay");
    a.session.file_name = Some("v1.2 notes".into());
    assert_eq!(super::export_stem(&a), "v1.2 notes", "only a document extension is dropped");
}
