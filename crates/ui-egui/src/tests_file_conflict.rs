//! Save-conflict behavior at the UI dispatch boundary, with original synthetic files only.
#![allow(clippy::unwrap_used)]
use super::*;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::channel;
use wordcraft_doc::Document;
use wordcraft_engine::io;

struct Scratch(PathBuf);
impl Scratch {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let p = std::env::temp_dir().join(format!("wordcraft-ui-conflict-{}-{}", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)));
        std::fs::create_dir(&p).unwrap();
        Self(p)
    }
    fn file(&self, name: &str) -> PathBuf {
        self.0.join(name)
    }
}
impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
fn app(d: &Scratch) -> WordApp {
    let p = d.file("original.docx");
    io::save_path(&p, &Document::from_text("Original")).unwrap();
    let mut a = WordApp::new(Session::new(Document::new()), Services::default());
    a.execute("file.open", json!({"path": p})).unwrap();
    a.execute("document.setText", json!({"text": "Local edit"})).unwrap();
    a
}
fn external(d: &Scratch) -> Vec<u8> {
    let b = io::save_bytes("external.docx", &Document::from_text("External version")).unwrap();
    std::fs::write(d.file("original.docx"), &b).unwrap();
    b
}
fn assert_local(a: &WordApp, d: &Scratch, b: &[u8]) {
    assert!(a.session.dirty);
    assert_eq!(a.session.doc.plain_text(wordcraft_doc::StoryRef::Body), "Local edit");
    assert_eq!(std::fs::read(d.file("original.docx")).unwrap(), b);
}
#[test]
fn scripts_get_error_user_gets_dialog_and_keep_editing_retains_both_versions() {
    let d = Scratch::new();
    let mut a = app(&d);
    let b = external(&d);
    assert!(a.execute("file.save", json!({})).is_err());
    assert!(a.dialog.is_none());
    assert!(a.run("file.save", json!({})).is_err());
    assert!(matches!(a.dialog, Some(dialogs::Dialog::SaveConflict { .. })));
    assert_local(&a, &d, &b);
    assert!(a.run("ui.saveConflict", json!({"answer": "overwrite"})).is_err());
    assert!(a.dialog.is_some());
    a.run("ui.saveConflict", json!({"answer": "cancel"})).unwrap();
    assert!(a.dialog.is_none());
    assert_local(&a, &d, &b);
}
#[test]
fn save_copy_picker_success_cancel_and_changed_document_are_bound_to_the_conflict() {
    for mode in ["copy", "cancel", "new-document", "same-path"] {
        let d = Scratch::new();
        let mut a = app(&d);
        let b = external(&d);
        let (tx, rx) = channel();
        let rx = std::cell::RefCell::new(Some(rx));
        a.services.file_dialog = Some(Box::new(move |request| {
            assert!(matches!(request, file_dialogs::FileDialogRequest::Save { name, dir: None } if name == "original copy.docx"));
            rx.borrow_mut().take().unwrap()
        }));
        assert!(a.run("file.save", json!({})).is_err());
        a.run("ui.saveConflict", json!({"answer": "copy"})).unwrap();
        assert!(a.file_dialog_open());
        let copy = d.file("copy.docx");
        if mode == "new-document" {
            a.execute("file.new", json!({})).unwrap();
        }
        tx.send(Ok(match mode {
            "cancel" => Vec::new(),
            "same-path" => vec![d.file("original.docx").display().to_string()],
            _ => vec![copy.display().to_string()],
        }))
        .unwrap();
        let result = a.poll_file_dialog().unwrap();
        if mode == "copy" {
            result.unwrap();
            assert!(!a.session.dirty);
            assert_eq!(a.session.path.as_ref(), Some(&copy));
            assert_eq!(io::open_path(&copy).unwrap().plain_text(wordcraft_doc::StoryRef::Body), "Local edit");
        } else {
            assert!(!copy.exists());
            if mode == "cancel" {
                result.unwrap();
                assert_local(&a, &d, &b);
            } else {
                assert!(result.is_err());
            }
            if mode == "same-path" {
                assert_local(&a, &d, &b);
                assert!(matches!(a.dialog, Some(dialogs::Dialog::SaveConflict { .. })));
            }
        }
        assert_eq!(std::fs::read(d.file("original.docx")).unwrap(), b);
    }
}
#[test]
fn autosave_pauses_after_conflict_and_pending_close_does_not_discard_edits() {
    let d = Scratch::new();
    let mut a = app(&d);
    a.execute("file.save", json!({})).unwrap();
    a.execute("document.setText", json!({"text": "Local edit"})).unwrap();
    let b = external(&d);
    a.autosave_tick(now_ms() + 10_000.0);
    assert!(!a.autosaves());
    assert!(matches!(a.dialog, Some(dialogs::Dialog::SaveConflict { .. })));
    assert_local(&a, &d, &b);
    a.run("ui.saveConflict", json!({"answer": "cancel"})).unwrap();
    a.autosave_tick(now_ms() + 20_000.0);
    assert!(a.dialog.is_none());
    assert_eq!(a.run("file.close", json!({})).unwrap()["pending"], "saveChanges");
    let error = a.run("ui.saveChanges", json!({"answer": "save"})).unwrap_err();
    assert!(error.contains("file changed or is unavailable"));
    assert!(
        matches!(&a.dialog, Some(dialogs::Dialog::SaveChanges { then, message, .. }) if then == "file.close" && message.contains("file changed or is unavailable"))
    );
    assert!(a.save_conflict.is_none());
    assert!(!a.quit_requested);
    assert_local(&a, &d, &b);
}

#[test]
fn dialog_buttons_keep_edits_and_render_selected_direction() {
    use egui_kittest::kittest::Queryable;
    let d = Scratch::new();
    let mut a = app(&d);
    let b = external(&d);
    a.run("file.save", json!({})).unwrap_err();
    let mut fonts = false;
    let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(1280.0, 800.0)).build_ui_state(
        move |ui, app: &mut WordApp| {
            let ctx = ui.ctx().clone();
            crate::i18n::set_current(crate::i18n::Lang::EN);
            if fonts {
                dialogs::show(app, &ctx);
            } else {
                crate::theme::install_fonts_for(&ctx, false);
                fonts = true;
            }
        },
        a,
    );
    h.run();
    h.get_by_label("Keep editing").click();
    h.run();
    assert!(h.state().dialog.is_none());
    assert_local(h.state(), &d, &b);
    h.state_mut().run("file.save", json!({})).unwrap_err();
    h.run();
    h.get_by_label("Save copy…").click();
    h.run();
    // This harness has no file picker, so Save Copy is cancelled without writing.
    assert!(h.state().dialog.is_none());
    assert_local(h.state(), &d, &b);
}

#[test]
fn save_copy_name_has_no_trailing_dot_without_an_extension() {
    assert_eq!(copy_name("Report.docx"), "Report copy.docx");
    assert_eq!(copy_name("Report"), "Report copy");
    assert_eq!(copy_name("Report."), "Report copy");
}

#[test]
fn busy_copy_picker_retains_conflict_and_can_be_retried() {
    use std::cell::RefCell;
    use std::collections::VecDeque;
    let d = Scratch::new();
    let mut a = app(&d);
    let b = external(&d);
    let (first_tx, first_rx) = channel();
    let (copy_tx, copy_rx) = channel();
    let receivers = RefCell::new(VecDeque::from([first_rx, copy_rx]));
    a.services.file_dialog = Some(Box::new(move |_| receivers.borrow_mut().pop_front().unwrap()));
    assert!(matches!(
        a.ask_file(
            file_dialogs::FileDialogRequest::Save { name: "Unrelated.docx".into(), dir: None },
            file_dialogs::AfterPick::Export { ext: "docx".into() }
        ),
        file_dialogs::Asked::Pending
    ));
    a.run("file.save", json!({})).unwrap_err();
    let result = a.run("ui.saveConflict", json!({"answer":"copy"}));
    assert!(result.is_err(), "Busy must not acknowledge a picker that never opened: {result:?}");
    assert!(matches!(a.dialog, Some(dialogs::Dialog::SaveConflict { .. })));
    assert_local(&a, &d, &b);
    first_tx.send(Ok(Vec::new())).unwrap();
    a.poll_file_dialog().unwrap().unwrap();
    assert_eq!(a.run("ui.saveConflict", json!({"answer":"copy"})).unwrap()["pending"], "saveCopy");
    let copy = d.file("retry.docx");
    copy_tx.send(Ok(vec![copy.display().to_string()])).unwrap();
    a.poll_file_dialog().unwrap().unwrap();
    assert_eq!(io::open_path(&copy).unwrap().plain_text(wordcraft_doc::StoryRef::Body), "Local edit");
    assert!(!a.session.dirty);
    assert_eq!(std::fs::read(d.file("original.docx")).unwrap(), b);
}

#[test]
fn autosave_conflict_retains_open_save_changes_and_its_close_command() {
    let d = Scratch::new();
    let mut a = app(&d);
    a.execute("file.save", json!({})).unwrap();
    a.execute("document.setText", json!({"text":"Local edit"})).unwrap();
    a.execute("file.autosave", json!({"value":true})).unwrap();
    let b = external(&d);
    assert_eq!(a.run("file.close", json!({})).unwrap()["pending"], "saveChanges");
    a.autosave_tick(now_ms() + 10_000.0);
    assert!(!a.autosaves());
    assert!(matches!(&a.dialog, Some(dialogs::Dialog::SaveChanges { then, .. }) if then == "file.close"));
    assert!(!a.quit_requested);
    assert_local(&a, &d, &b);
    assert!(a.run("ui.saveChanges", json!({"answer":"save"})).is_err());
    assert!(
        matches!(&a.dialog, Some(dialogs::Dialog::SaveChanges { then, message, .. }) if then == "file.close" && message.contains("file changed or is unavailable"))
    );
    a.run("ui.saveChanges", json!({"answer":"cancel"})).unwrap();
    assert!(a.dialog.is_none());
    assert!(!a.quit_requested);
    assert_local(&a, &d, &b);
}

#[test]
fn native_byte_open_keeps_its_baseline_while_save_changes_is_pending() {
    let dir = Scratch::new();
    let path = dir.file("original.docx");
    let a = wordcraft_engine::io::save_bytes("original.docx", &wordcraft_doc::Document::from_text("Original A")).unwrap();
    let b = wordcraft_engine::io::save_bytes("external.docx", &wordcraft_doc::Document::from_text("External B")).unwrap();
    std::fs::write(&path, a).unwrap();
    let mut app = WordApp::new(Session::new(Document::new()), Services::default());
    app.run("text.insert", json!({"text":"Previous unsaved draft"})).unwrap();
    let result = app.open_native_path(&path).unwrap();
    assert!(result.get("pending").is_some());
    assert!(app.open_native_path(&dir.file("missing.docx")).is_err());
    std::fs::write(&path, &b).unwrap();
    app.run("ui.saveChanges", json!({"answer":"dontSave"})).unwrap();
    assert_eq!(app.session.path.as_deref(), Some(path.as_path()));
    app.run("text.insert", json!({"text":"Local edits"})).unwrap();
    assert!(app.run("file.save", json!({})).is_err());
    assert_eq!(std::fs::read(&path).unwrap(), b);
    assert!(app.session.dirty);
}

#[cfg(target_os = "linux")]
#[test]
fn abandoned_native_opens_release_handles_but_pending_dialogs_keep_them() {
    let d = Scratch::new();
    let handles = || {
        std::fs::read_dir("/proc/self/fd")
            .unwrap()
            .filter_map(Result::ok)
            .filter_map(|entry| std::fs::read_link(entry.path()).ok())
            .filter(|path| path.starts_with(&d.0))
            .count()
    };
    let mut a = WordApp::new(Session::new(Document::new()), Services::default());
    for i in 0..100 {
        let path = d.file(&format!("malformed-{i}.docx"));
        std::fs::write(&path, b"Not a document package").unwrap();
        assert!(a.open_native_path(&path).is_err());
    }
    assert_eq!(handles(), 0, "definitive failed opens release their read handles");
    let path = d.file("protected.docx");
    let bytes =
        io::save_bytes_with("protected.docx", &Document::from_text("Protected original"), Some("synthetic-password"), &io::ExportOptions::default())
            .unwrap();
    std::fs::write(&path, &bytes).unwrap();
    a.run("text.insert", json!({"text":"Unsaved draft"})).unwrap();
    assert_eq!(a.open_native_path(&path).unwrap()["pending"], "saveChanges");
    let mut h = egui_kittest::Harness::builder().build_ui_state(|ui, app: &mut WordApp| app.logic(ui.ctx()), a);
    h.run();
    assert_eq!(handles(), 1);
    h.state_mut().run("ui.saveChanges", json!({"answer":"cancel"})).unwrap();
    h.run();
    assert_eq!(handles(), 0);
    assert!(h.state().session.dirty);
    assert_eq!(h.state().session.doc.plain_text(wordcraft_doc::StoryRef::Body), "Unsaved draft");
    h.state_mut().execute("file.new", json!({})).unwrap();
    assert_eq!(h.state_mut().open_native_path(&path).unwrap()["pending"], "password");
    h.run();
    assert_eq!(handles(), 1);
    h.state_mut().run("ui.closeDialog", json!({})).unwrap();
    h.run();
    assert_eq!(handles(), 0);
    assert_eq!(std::fs::read(&path).unwrap(), bytes);
}

#[test]
fn native_open_baseline_survives_the_save_changes_save_as_continuation() {
    let d = Scratch::new();
    let path = d.file("next.docx");
    io::save_path(&path, &Document::from_text("Next original")).unwrap();
    let mut a = WordApp::new(Session::new(Document::new()), Services::default());
    a.run("text.insert", json!({"text":"Prior draft"})).unwrap();
    a.open_native_path(&path).unwrap();
    let (tx, rx) = channel();
    let receiver = std::cell::RefCell::new(Some(rx));
    a.services.file_dialog = Some(Box::new(move |_| receiver.borrow_mut().take().unwrap()));
    assert_eq!(a.run("ui.saveChanges", json!({"answer":"save"})).unwrap()["pending"], "saveAs");
    let mut h = egui_kittest::Harness::builder().build_ui_state(|ui, app: &mut WordApp| app.logic(ui.ctx()), a);
    h.run();
    let prior = d.file("prior.docx");
    tx.send(Ok(vec![prior.display().to_string()])).unwrap();
    h.run();
    assert_eq!(h.state().session.path.as_ref(), Some(&path));
    assert_eq!(io::open_path(&prior).unwrap().plain_text(wordcraft_doc::StoryRef::Body), "Prior draft");
    let external = io::save_bytes("external.docx", &Document::from_text("External next")).unwrap();
    std::fs::write(&path, &external).unwrap();
    h.state_mut().run("text.insert", json!({"text":"Local next"})).unwrap();
    assert!(h.state_mut().run("file.save", json!({})).is_err());
    assert_eq!(std::fs::read(path).unwrap(), external);
    assert!(h.state().session.dirty);
}

/// Native Windows CI runs this through the existing app/UI test gate.
#[cfg(windows)]
#[test]
fn native_file_versions_leave_windows_destinations_closed_between_commands() {
    use std::fs::OpenOptions;
    use std::os::windows::fs::OpenOptionsExt;
    let d = Scratch::new();
    let mut a = app(&d);
    let path = d.file("original.docx");
    for text in ["First safe save", "Second safe save"] {
        // A non-sharing open independently detects any retained destination handle.
        let file = OpenOptions::new().read(true).share_mode(0).open(&path).unwrap();
        drop(file);
        a.execute("document.setText", json!({"text": text})).unwrap();
        a.execute("file.save", json!({})).unwrap();
        assert_eq!(io::open_path(&path).unwrap().plain_text(wordcraft_doc::StoryRef::Body), text);
    }
    let file = OpenOptions::new().read(true).share_mode(0).open(&path).unwrap();
    drop(file);
    a.execute("document.setText", json!({"text": "Local edit"})).unwrap();
    let external = external(&d);
    assert!(a.execute("file.save", json!({})).is_err());
    assert_local(&a, &d, &external);
}
