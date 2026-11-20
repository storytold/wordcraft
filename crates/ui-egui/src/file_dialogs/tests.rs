use std::path::PathBuf;
use std::sync::Arc;

use craft::host::{PickKind, PickRequest, Picked, ScriptedFiles};

use super::*;
use crate::Services;
use wordcraft_engine::Session;

const UNSAVED: &str = "My unsaved novel chapter";

/// Original Word 97 compound file: one compressed text piece, built from MS-DOC's FIB/Clx.
fn legacy_word_bytes() -> Vec<u8> {
    use std::io::{Cursor, Write};
    let text = b"Legacy contract\r";
    let mut word = vec![0u8; 0x600];
    for (at, value) in [(0, 0xA5ECu16), (2, 0x00C1), (0x0A, 0x1000), (0x0C, 0x00BF), (0x20, 14), (0x3E, 22), (0x98, 0x005D)] {
        word[at..at + 2].copy_from_slice(&value.to_le_bytes());
    }
    word[0x40..0x44].copy_from_slice(&0x1000u32.to_le_bytes());
    word[0x4C..0x50].copy_from_slice(&(text.len() as u32).to_le_bytes());
    let mut clx = vec![2];
    clx.extend_from_slice(&16u32.to_le_bytes());
    clx.extend_from_slice(&0u32.to_le_bytes());
    clx.extend_from_slice(&(text.len() as u32).to_le_bytes());
    clx.extend_from_slice(&[0, 0]);
    clx.extend_from_slice(&(0x4000_0000u32 | (2 * 0x600)).to_le_bytes());
    clx.extend_from_slice(&[0, 0]);
    let pair = 0x9A + 33 * 8;
    word[pair..pair + 4].copy_from_slice(&0x40u32.to_le_bytes());
    word[pair + 4..pair + 8].copy_from_slice(&(clx.len() as u32).to_le_bytes());
    word.extend_from_slice(text);
    let mut table = vec![0; 0x40];
    table.extend_from_slice(&clx);
    let mut file = cfb::CompoundFile::create(Cursor::new(Vec::new())).unwrap();
    for (name, data) in [("/WordDocument", word), ("/0Table", table)] {
        file.create_stream(name).unwrap().write_all(&data).unwrap();
    }
    file.into_inner().into_inner()
}

/// An app over `files`.
fn app_over(files: &Arc<ScriptedFiles>, document: wordcraft_doc::Document) -> WordApp {
    // PLAN: assumed `WordApp::with_host(Arc<Host>) -> Self` (app.H1, S-L103)
    let host =
        craft::host::Host::builder(craft::host::AppInfo { id: "wordcraft-test", name: "WordCraft", version: "0" }).files(files.clone()).build();
    WordApp::new(Session::new(document), Services::default()).with_host(Arc::new(host))
}

/// An app whose file service holds every pick until the test answers it.
fn async_app() -> (WordApp, Arc<ScriptedFiles>) {
    let files = Arc::new(ScriptedFiles::held());
    (app_over(&files, wordcraft_doc::Document::new()), files)
}

/// The answer for `paths` (none: cancelled).
fn picked(paths: &[&str]) -> Picked {
    if paths.is_empty() { Picked::Cancelled } else { Picked::Paths(paths.iter().map(PathBuf::from).collect()) }
}

/// Answer the open dialog (none: cancelled).
fn answer(files: &ScriptedFiles, picked_path: Option<&str>) {
    answer_all(files, picked_path.as_slice());
}

/// Answer the open dialog with several picked paths (none: cancelled).
fn answer_all(files: &ScriptedFiles, paths: &[&str]) {
    files.answer(picked(paths));
}

/// The `n`th request asked so far.
fn request(files: &ScriptedFiles, n: usize) -> PickRequest {
    files.requests().get(n).cloned().unwrap()
}

/// The status line's text.
fn status_text(a: &WordApp) -> String {
    a.status.as_ref().map(|s| s.text.clone()).unwrap_or_default()
}

/// #2445: Save on an imported binary Word file asks for DOCX, preserving the import on Cancel.
#[test]
fn imported_word_save_asks_for_docx_and_preserves_the_original() {
    for ext in ["doc", "DOT", "DOC", "dot"] {
        let dir = scratch(&format!("imported-{ext}"));
        let original = dir.join(format!("contract.{ext}"));
        let bytes = legacy_word_bytes();
        std::fs::write(&original, &bytes).unwrap();
        let (mut a, files) = async_app();
        a.run("file.open", json!({"path": original.to_string_lossy()})).unwrap();
        assert!(a.session.doc.plain_text(wordcraft_doc::StoryRef::Body).contains("Legacy contract"));
        a.run("text.insert", json!({"text": "Revised "})).unwrap();
        let draft = a.session.doc.clone();
        let revision = a.session.rev();
        let result = a.run("file.save", json!({})).unwrap();
        assert_eq!(result["pending"], "saveAs");
        let asked = request(&files, 0);
        assert_eq!(asked.kind, PickKind::Save);
        assert_eq!(asked.file_name.as_deref(), Some("contract.docx"));
        assert_eq!(asked.directory.as_deref(), Some(dir.as_path()));
        answer(&files, None);
        assert_eq!(a.poll_file_dialog().unwrap().unwrap()["saved"], false);
        assert_eq!(a.session.doc, draft);
        assert_eq!(a.session.rev(), revision);
        assert!(a.session.dirty);
        assert_eq!(a.session.path.as_deref(), Some(original.as_path()));
        assert_eq!(std::fs::read(&original).unwrap(), bytes);

        a.run("file.save", json!({})).unwrap();
        let saved = dir.join("revised.docx");
        answer(&files, Some(&saved.to_string_lossy()));
        assert_eq!(a.poll_file_dialog().unwrap().unwrap()["saved"], true);
        assert!(!a.session.dirty);
        assert_eq!(a.session.path.as_deref(), Some(saved.as_path()));
        assert!(saved_text(&saved).contains("Revised Legacy contract"));
        assert_eq!(std::fs::read(&original).unwrap(), bytes);
        // The new supported path saves directly, without another chooser.
        let asked = files.requests().len();
        a.run("text.insert", json!({"text": "Again "})).unwrap();
        assert_eq!(a.run("file.save", json!({})).unwrap()["saved"], true);
        assert_eq!(files.requests().len(), asked);
        assert!(saved_text(&saved).contains("Again "));
        let _ = std::fs::remove_dir_all(dir);
    }
}

#[test]
fn imported_word_close_waits_for_save_as_and_keeps_failed_or_cancelled_drafts() {
    let dir = scratch("imported-close");
    let original = dir.join("contract.doc");
    let bytes = legacy_word_bytes();
    std::fs::write(&original, &bytes).unwrap();
    let (mut a, files) = async_app();
    a.run("file.open", json!({"path": original.to_string_lossy()})).unwrap();
    a.run("text.insert", json!({"text": "Revised "})).unwrap();
    let draft = a.session.doc.clone();

    a.run("file.close", json!({})).unwrap();
    assert_eq!(a.run("ui.saveChanges", json!({"answer": "save"})).unwrap()["pending"], "saveAs");
    assert!(a.has_document() && !a.quit_requested);
    answer(&files, Some(&dir.join("missing").join("contract.docx").to_string_lossy()));
    let error = a.poll_file_dialog().unwrap().unwrap_err();
    assert!(error.contains("still open"), "{error}");
    // PLAN(shell-w8): sh.A7 `WordApp::closing: Option<Closing>`; the failed write reopens the prompt.
    assert!(a.closing.as_ref().is_some_and(|c| c.prompt.is_some()), "the Save Changes prompt is open again");
    assert!(a.has_document() && a.session.dirty && !a.quit_requested);
    assert_eq!(a.session.path.as_deref(), Some(original.as_path()));
    assert_eq!(std::fs::read(&original).unwrap(), bytes);
    assert_eq!(a.session.doc.plain_text(wordcraft_doc::StoryRef::Body), draft.plain_text(wordcraft_doc::StoryRef::Body));

    a.run("ui.saveChanges", json!({"answer": "save"})).unwrap();
    answer(&files, None);
    assert_eq!(a.poll_file_dialog().unwrap().unwrap()["done"], false);
    assert!(a.closing.is_none(), "a cancelled Save As ends the close");
    assert!(a.has_document() && a.session.dirty && !a.quit_requested);
    assert_eq!(a.session.path.as_deref(), Some(original.as_path()));

    a.run("file.close", json!({})).unwrap();
    a.run("ui.saveChanges", json!({"answer": "save"})).unwrap();
    let saved = dir.join("contract.docx");
    answer(&files, Some(&saved.to_string_lossy()));
    assert_eq!(a.poll_file_dialog().unwrap().unwrap()["done"], true);
    // File › Close keeps the window (#1712): the document closes once the DOCX save succeeds.
    assert!(!a.has_document() && !a.quit_requested, "Close runs only after the DOCX save succeeds");
    assert!(saved_text(&saved).contains("Revised Legacy contract"));
    assert_eq!(std::fs::read(&original).unwrap(), bytes);
    let _ = std::fs::remove_dir_all(dir);
}

/// A late Save As answer belongs to the imported document that opened the picker: once that
/// document is closed, it writes nothing.
#[test]
fn imported_word_stale_save_as_never_writes_the_replacement_document() {
    for close in [false, true] {
        let dir = scratch(if close { "imported-stale-close" } else { "imported-stale-save" });
        let original = dir.join("contract.doc");
        let bytes = legacy_word_bytes();
        std::fs::write(&original, &bytes).unwrap();
        let destination = dir.join("existing.docx");
        let sentinel = b"Existing destination must survive";
        std::fs::write(&destination, sentinel).unwrap();
        let (mut a, files) = async_app();
        a.run("file.open", json!({"path": original})).unwrap();
        typed(&mut a);
        if close {
            a.run("file.close", json!({})).unwrap();
            a.run("ui.saveChanges", json!({"answer": "save"})).unwrap();
        } else {
            a.run("file.save", json!({})).unwrap();
        }
        assert!(a.file_dialog_open());
        let previous = a.session.document_id();
        // Automation bypasses the user prompt and can close a document during a picker.
        a.execute("file.closeDocument", json!({"force": true})).unwrap();
        typed(&mut a);
        assert_ne!(previous, a.session.document_id());
        let draft = a.session.doc.clone();
        let revision = a.session.rev();
        answer(&files, Some(&destination.to_string_lossy()));
        let _ = a.poll_file_dialog().unwrap();
        assert!(std::fs::read(&destination).unwrap() == sentinel, "stale picker overwrote the destination");
        assert_eq!(std::fs::read(&original).unwrap(), bytes);
        assert_eq!(a.session.doc, draft);
        assert_eq!(a.session.rev(), revision);
        assert!(a.session.dirty && a.session.path.is_none() && !a.quit_requested);
    }
}

#[test]
fn imported_word_programmatic_and_explicit_saves_never_open_dialogs() {
    let dir = scratch("imported-explicit");
    let original = dir.join("contract.doc");
    let bytes = legacy_word_bytes();
    std::fs::write(&original, &bytes).unwrap();
    let (mut a, files) = async_app();
    a.run("file.open", json!({"path": original.to_string_lossy()})).unwrap();
    a.run("text.insert", json!({"text": "Revised "})).unwrap();
    let error = a.execute("file.save", json!({})).unwrap_err();
    assert!(error.contains("can't save as `.doc`"), "{error}");
    let error = a.run("file.save", json!({"path": original.to_string_lossy()})).unwrap_err();
    assert!(error.contains("can't save as `.doc`"), "{error}");
    assert!(files.requests().is_empty());
    assert!(a.session.dirty);
    assert_eq!(a.session.path.as_deref(), Some(original.as_path()));
    assert_eq!(std::fs::read(&original).unwrap(), bytes);
    let _ = std::fs::remove_dir_all(dir);
}

fn typed(a: &mut WordApp) {
    a.run("text.insert", json!({"text": UNSAVED})).unwrap();
    assert!(a.session.dirty);
}

fn body_text(a: &WordApp) -> String {
    a.session.doc.plain_text(wordcraft_doc::StoryRef::Body)
}

fn scratch(name: &str) -> std::path::PathBuf {
    let dir = std::path::absolute(std::env::temp_dir().join(format!("wordcraft-ui-dialogs-{name}-{}", std::process::id()))).expect("temp dir");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn saved_text(path: &std::path::Path) -> String {
    wordcraft_engine::io::open_path(path).unwrap().plain_text(wordcraft_doc::StoryRef::Body)
}

/// Save as Word Template (#2852) suggests a .dotx in the user templates folder, which is made
/// on first use; the picked path is saved as a template. A folder named in File › Options
/// replaces the app's own.
#[test]
fn save_as_template_starts_in_the_templates_folder() {
    let dir = scratch("save-template");
    let own = dir.join("Templates");
    let (mut a, files) = async_app();
    a.services.templates_dir = Some(own.clone());
    typed(&mut a);
    let r = a.run("ui.saveAsTemplate", json!({})).unwrap();
    assert!(own.is_dir(), "the app's templates folder is made on first use");
    assert_eq!(r["dir"], json!(own.to_string_lossy()));
    let asked = request(&files, 0);
    assert_eq!(asked.file_name.as_deref(), Some("Document1.dotx"));
    assert_eq!(asked.directory.as_deref(), Some(own.as_path()));
    let path = own.join("Memo.dotx");
    answer(&files, Some(&path.to_string_lossy()));
    let ctx = egui::Context::default();
    ctx.run_ui(egui::RawInput::default(), |ui| a.logic(ui.ctx())).drop_without_applying_deltas();
    assert!(path.is_file(), "saved as a template");
    assert_eq!(saved_text(&path), UNSAVED);
    let listed = wordcraft_engine::cmd::user_templates::list_templates(&own.to_string_lossy()).unwrap();
    assert_eq!(listed.templates.iter().map(|t| t.label.as_str()).collect::<Vec<_>>(), ["Memo"]);

    // A folder chosen in Options wins.
    let chosen = dir.join("Chosen");
    std::fs::create_dir_all(&chosen).unwrap();
    a.run("file.options", json!({"userTemplatesFolder": chosen.to_string_lossy()})).unwrap();
    assert_eq!(a.user_templates_dir(), Some(chosen.clone()));
    a.run("ui.saveAsTemplate", json!({})).unwrap();
    let asked = request(&files, 1);
    assert_eq!(asked.file_name.as_deref(), Some("Memo.dotx"));
    assert_eq!(asked.directory.as_deref(), Some(chosen.as_path()));
    answer(&files, None);
    let _ = std::fs::remove_dir_all(&dir);
}

/// #4143: Save As starts in the folder of the open file, not in the home folder; a document
/// that was never saved has no folder to offer.
#[test]
fn save_as_starts_in_the_folder_of_the_open_file() {
    let dir = scratch("save-as-folder");
    let opened = dir.join("one.docx");
    wordcraft_engine::Session::new(wordcraft_doc::Document::from_text("original"))
        .run("file.save", &json!({"path": opened.to_string_lossy()}))
        .unwrap();
    let (mut a, files) = async_app();
    a.services.templates_dir = Some(dir.join("Templates"));
    a.run("file.open", json!({"path": opened.to_string_lossy()})).unwrap();
    a.save_as_dialog();
    let asked = request(&files, 0);
    assert_eq!(asked.file_name.as_deref(), Some("one.docx"));
    assert_eq!(asked.directory.as_deref(), Some(dir.as_path()));
    answer(&files, None);

    let (mut fresh, files) = async_app();
    typed(&mut fresh);
    fresh.save_as_dialog();
    let asked = request(&files, 0);
    assert_eq!(asked.file_name, Some(format!("{UNSAVED}.docx")));
    assert_eq!(asked.directory, None);
    answer(&files, None);
    let _ = std::fs::remove_dir_all(&dir);
}

/// A file opened by a relative path (`wordcraft one.docx`) has an empty parent: Save As starts
/// in the working folder, not with an empty folder the dialog would drop.
#[test]
fn save_as_starts_in_the_working_folder_for_a_relative_path() {
    let (mut a, files) = async_app();
    typed(&mut a);
    a.session.path = Some(std::path::PathBuf::from("one.docx"));
    a.save_as_dialog();
    let asked = request(&files, 0);
    assert_eq!(asked.file_name.as_deref(), Some("one.docx"));
    assert_eq!(asked.directory, Some(std::env::current_dir().unwrap()));
    answer(&files, None);
}

/// #689: Save As honours the filename typed in the dialog, not just the folder: a document
/// opened as `a/one.docx` and saved as `b/two.docx` is written there and takes that path.
#[test]
fn save_as_with_a_new_filename_saves_under_it() {
    let dir = scratch("rename");
    let opened = dir.join("one.docx");
    let picked_path = dir.join("sub").join("two.docx");
    std::fs::create_dir_all(dir.join("sub")).unwrap();
    wordcraft_engine::Session::new(wordcraft_doc::Document::from_text("original"))
        .run("file.save", &json!({"path": opened.to_string_lossy()}))
        .unwrap();
    let (mut a, files) = async_app();
    a.run("file.open", json!({"path": opened.to_string_lossy()})).unwrap();
    assert_eq!(a.session.path.as_deref(), Some(opened.as_path()));
    a.save_as_dialog();
    answer(&files, Some(&picked_path.to_string_lossy()));
    let r = a.poll_file_dialog().unwrap().unwrap();
    assert_eq!(r, json!({"saved": true}), "the save-as succeeded");
    assert_eq!(a.session.path.as_deref(), Some(picked_path.as_path()), "the picked filename is the document's path");
    assert!(picked_path.is_file(), "the file is written under the picked name");
    assert!(saved_text(&picked_path).contains("original"));
    // F12 (file.saveAs without a path) lands in the same dialog, suggesting the new name.
    let r = a.run("file.saveAs", json!({})).unwrap();
    assert_eq!(r, json!({"saved": false}));
    assert!(a.file_dialog_open(), "F12's saveAs request opens the dialog");
    let asked = request(&files, 1);
    assert_eq!(asked.file_name.as_deref(), Some("two.docx"));
    assert_eq!(asked.directory.as_deref(), Some(dir.join("sub").as_path()));
    let _ = std::fs::remove_dir_all(&dir);
}

/// Backstage File > Save > Save As page > Browse… picks a folder that can't be written to
/// (missing, no permission...): the failure reaches the status bar, and the document (and its
/// unsaved text) stays open regardless.
#[test]
fn a_failed_save_as_says_so_and_keeps_the_document() {
    let dir = scratch("write-fails");
    let bogus = dir.join("does-not-exist").join("out.docx");
    let (mut a, files) = async_app();
    typed(&mut a);
    a.run("ui.backstage", json!({"value": true, "page": "saveAs"})).unwrap();
    a.save_as_dialog();
    answer(&files, Some(&bogus.to_string_lossy()));
    let r = a.poll_file_dialog().unwrap().unwrap();
    assert_eq!(r["saved"], false, "the write failed");
    assert!(!bogus.exists());
    assert!(a.ui.backstage, "backstage stays open: nothing was actually saved");
    assert!(a.has_document() && body_text(&a).contains(UNSAVED), "the document and its text are kept");
    assert!(!status_text(&a).is_empty(), "the failure is no longer silent");
    let _ = std::fs::remove_dir_all(&dir);
}

/// The AutoSave switch's own Save As goes through the same `AfterPick::SaveAs` arm as
/// Backstage's, under `AfterSave::AutoSave`: a write failure there must reach the status bar
/// too, and AutoSave must stay off rather than claim coverage it doesn't have.
#[test]
fn a_failed_autosave_switch_save_as_says_so_and_leaves_autosave_off() {
    let dir = scratch("autosave-switch-write-fails");
    let bogus = dir.join("does-not-exist").join("out.docx");
    let (mut a, files) = async_app();
    typed(&mut a);
    assert!(matches!(a.autosave_block(), Some(crate::AutoSaveBlock::Unsaved)), "a new document blocks AutoSave until saved");
    assert!(!a.autosaves(), "blocked: AutoSave isn't covering the document yet");
    a.save_as(AfterSave::AutoSave);
    answer(&files, Some(&bogus.to_string_lossy()));
    let r = a.poll_file_dialog().unwrap().unwrap();
    assert_eq!(r["saved"], false, "the write failed");
    assert!(!a.autosaves(), "still blocked: the save that would have lifted it never happened");
    assert!(a.session.path.is_none(), "unsaved: the write never actually landed");
    assert!(a.has_document() && body_text(&a).contains(UNSAVED), "the document and its text are kept");
    assert!(!status_text(&a).is_empty(), "the failure is no longer silent");
    let _ = std::fs::remove_dir_all(&dir);
}

/// While a dialog is open, more Save As / Open / Export / Picture clicks don't open a second one.
#[test]
fn a_second_dialog_while_one_is_open_is_ignored() {
    let (mut a, files) = async_app();
    a.save_as_dialog();
    a.save_as_dialog();
    a.open_dialog();
    a.export_dialog("pdf");
    a.pick_picture();
    assert_eq!(files.requests().len(), 1, "only the first dialog was shown");
    answer(&files, None);
    a.poll_file_dialog();
    assert!(!a.file_dialog_open());
    a.open_dialog();
    assert_eq!(files.requests().len(), 2, "a new dialog once the first was answered");
    let last = request(&files, 1);
    assert_eq!((last.kind, last.purpose.as_str()), (PickKind::Open, "document"));
}

/// A failed web save must not stamp a save that never reached its destination.
#[test]
fn failed_web_saves_restore_the_save_stamps() {
    for id in ["file.save", "file.saveAs"] {
        for (name, dirty) in [("draft.unsupported", true), ("draft.docx", true), ("draft.docx", false)] {
            // PLAN(shell-w6): the web save is a `Files::save` ticket answered `Picked::Failed`.
            let files = Arc::new(ScriptedFiles::answering([Picked::Failed("host refused".into())]).in_browser());
            let mut a = app_over(&files, wordcraft_doc::Document::from_text(UNSAVED));
            a.session.dirty = dirty;
            a.session.path = Some("original.docx".into());
            a.session.view.zoom = 1.37;
            a.session.doc.core.created.clear();
            a.session.doc.core.modified = "2020-01-02T03:04:05Z".into();
            a.session.doc.core.last_modified_by = "Previous author".into();
            a.session.doc.core.revision = 7;
            let path_before = a.session.path.clone();
            let core_before = a.session.doc.core.clone();
            let zoom_before = a.session.doc.settings.zoom;
            let undo_before = a.session.undo_depth();
            // A name no format writes fails at once; a download the host refuses fails on its ticket, which
            // the next frame's `settle_saves` reads.
            let started = a.run(id, json!({"path": name}));
            if name == "draft.unsupported" {
                assert!(started.is_err(), "{id}: {name}");
            } else {
                assert!(started.is_ok_and(|v| v["downloaded"] == name), "{id}: {name}");
                a.settle_saves();
                assert!(status_text(&a).contains("host refused"), "{id}: {name}: {}", status_text(&a));
            }
            assert_eq!(a.session.doc.core, core_before, "{id}: {name}");
            assert_eq!(a.session.doc.settings.zoom, zoom_before, "{id}: {name}");
            assert_eq!(a.session.undo_depth(), undo_before, "{id}: {name}");
            assert_eq!(a.session.dirty, dirty, "{id}: {name}");
            assert_eq!(a.session.path, path_before, "{id}: {name}");
        }
        // The browser takes the download: the scripted host files its bytes under the name it answers with.
        let files = Arc::new(ScriptedFiles::answering([Picked::Paths(vec![PathBuf::from("draft.docx")])]).in_browser());
        let mut a = app_over(&files, wordcraft_doc::Document::from_text(UNSAVED));
        a.session.dirty = true;
        let revision_before = a.session.doc.core.revision;
        assert_eq!(a.run(id, json!({"path": "draft.docx"})).unwrap()["downloaded"], "draft.docx");
        assert!(files.file(std::path::Path::new("draft.docx")).is_some_and(|b| b.starts_with(b"PK")));
        assert_eq!(a.session.doc.core.revision, revision_before.saturating_add(1));
        assert_eq!(a.session.doc.core.last_modified_by, a.session.author);
        assert!(!a.session.doc.core.created.is_empty() && !a.session.doc.core.modified.is_empty());
        assert!(!a.session.dirty);
    }
}

/// An explicit web PDF export keeps an existing PDF suffix regardless of ASCII case.
#[test]
fn web_pdf_export_keeps_an_existing_suffix_in_any_case() {
    let files = Arc::new(ScriptedFiles::new().in_browser());
    let mut a = app_over(&files, wordcraft_doc::Document::new());
    for (name, expected) in [
        ("Report.pdf", "Report.pdf"),
        ("Report.PDF", "Report.PDF"),
        ("Résumé.PdF", "Résumé.PdF"),
        ("😀.PDF", "😀.PDF"),
        (".hidden.PDF", ".hidden.PDF"),
        ("Report..PDF", "Report..PDF"),
        ("é", "é.pdf"),
        ("😀", "😀.pdf"),
        ("Report.pdf.exe", "Report.pdf.exe.pdf"),
        ("Report.ＰＤＦ", "Report.ＰＤＦ.pdf"),
        ("Report.pdf.", "Report.pdf..pdf"),
        ("Report", "Report.pdf"),
    ] {
        // The browser takes the download: the scripted host files its bytes under the name it answers with.
        files.queue(Picked::Paths(vec![PathBuf::from(expected)]));
        let result = a.execute("file.exportPdf", json!({"path": name})).unwrap();
        assert_eq!(result["downloaded"], expected, "{name}");
        assert!(files.file(std::path::Path::new(expected)).is_some_and(|b| b.starts_with(b"%PDF")), "{name}");
    }
}

/// Open, Save As and Export PNG from the command search (no path) ask for a file the same way.
#[test]
fn file_commands_without_a_path_ask_for_one() {
    let (mut a, files) = async_app();
    a.run("file.open", json!({})).unwrap();
    let r = request(&files, 0);
    assert_eq!((r.kind, r.purpose.as_str()), (PickKind::Open, "document"));
    // A second dialog can't open while one is up, so cancel before the next command.
    answer(&files, None);
    a.poll_file_dialog();
    a.run("file.saveAs", json!({})).unwrap();
    assert_eq!(request(&files, 1).kind, PickKind::Save);
    answer(&files, None);
    a.poll_file_dialog();
    a.run("file.exportPng", json!({})).unwrap();
    let r = request(&files, 2);
    assert_eq!(r.kind, PickKind::Save);
    assert!(r.file_name.as_deref().is_some_and(|n| n.ends_with(".png")));
}

/// One row per entry point: what it asks for, with which file types and name.
#[test]
fn each_purpose_asks_with_its_filters_and_name() {
    struct Row {
        what: &'static str,
        ask: fn(&mut WordApp),
        kind: PickKind,
        purpose: &'static str,
        /// The extension the name ends with and a filter offers; `None` for an Open.
        ext: Option<&'static str>,
        directory: bool,
    }
    let rows = [
        Row { what: "open", ask: |a| a.open_dialog(), kind: PickKind::Open, purpose: "document", ext: None, directory: false },
        Row { what: "picture", ask: |a| a.pick_picture(), kind: PickKind::Open, purpose: "picture", ext: None, directory: false },
        Row { what: "font", ask: |a| a.pick_font(), kind: PickKind::OpenMany, purpose: "font", ext: None, directory: false },
        Row {
            what: "recipients",
            ask: |a| {
                let _ = a.run("ui.openRecipientList", json!({}));
            },
            kind: PickKind::Open,
            purpose: "recipients",
            ext: None,
            directory: false,
        },
        Row { what: "save as", ask: |a| a.save_as_dialog(), kind: PickKind::Save, purpose: "document", ext: Some("docx"), directory: false },
        Row { what: "export", ask: |a| a.export_dialog("pdf"), kind: PickKind::Save, purpose: "export", ext: Some("pdf"), directory: false },
        Row { what: "print pdf", ask: |a| a.print_pdf_dialog(), kind: PickKind::Save, purpose: "export", ext: Some("pdf"), directory: false },
        Row {
            what: "template",
            ask: |a| {
                a.services.templates_dir = Some(scratch("purpose-template").join("Templates"));
                a.save_as_template_dialog();
            },
            kind: PickKind::Save,
            purpose: "template",
            ext: Some("dotx"),
            directory: true,
        },
    ];
    for row in rows {
        let (mut a, files) = async_app();
        (row.ask)(&mut a);
        let r = request(&files, 0);
        assert_eq!(r.kind, row.kind, "{}", row.what);
        assert_eq!(r.purpose, row.purpose, "{}", row.what);
        assert!(!r.filters.is_empty(), "{} offers file types", row.what);
        assert_eq!(r.directory.is_some(), row.directory, "{}", row.what);
        match row.ext {
            Some(ext) => {
                let name = r.file_name.as_deref().unwrap_or_default();
                assert!(name.ends_with(&format!(".{ext}")), "{}: {name}", row.what);
                assert!(r.filters.iter().any(|f| f.extensions.iter().any(|e| e == ext)), "{} offers .{ext}", row.what);
            }
            None => assert!(r.file_name.is_none(), "{} suggests no name", row.what),
        }
        assert!(!r.fallback, "{} does not force the chooser", row.what);
    }
    // Answered: the export writes a PDF at the picked path.
    let (mut a, files) = async_app();
    a.export_dialog("pdf");
    let dir = scratch("export-pdf");
    let path = dir.join("out.pdf");
    answer(&files, Some(&path.to_string_lossy()));
    let r = a.poll_file_dialog().unwrap().unwrap();
    assert_eq!(r["path"], json!(path.to_string_lossy().to_string()));
    assert!(std::fs::read(&path).unwrap().starts_with(b"%PDF"), "a PDF is written to the picked path");
    let _ = std::fs::remove_dir_all(&dir);
}

/// `with_fallback_chooser(true)` and `ui.fileBrowser` set the request's `fallback`; nothing else does.
#[cfg(not(target_arch = "wasm32"))]
#[test]
fn a_forced_pick_sets_the_fallback_request_field() {
    let (mut a, files) = async_app();
    a.open_dialog();
    assert!(!request(&files, 0).fallback);
    answer(&files, None);
    a.poll_file_dialog();
    // PLAN(shell-w6): ui.fileBrowser
    a.run("ui.fileBrowser", json!({"purpose": "document"})).unwrap();
    assert!(request(&files, 1).fallback, "ui.fileBrowser forces the chooser");

    let files = Arc::new(ScriptedFiles::held());
    let mut b = app_over(&files, wordcraft_doc::Document::new()).with_fallback_chooser(true);
    b.open_dialog();
    assert!(request(&files, 0).fallback, "the forced chooser reaches every request");
}

/// An export waiting on a system dialog that never showed up keeps its format when Browse
/// in WordCraft… takes over, instead of starting over on PDF.
#[cfg(not(target_arch = "wasm32"))]
#[test]
fn browse_in_wordcraft_keeps_a_pending_exports_format() {
    let (mut a, files) = async_app();
    a.export_dialog("rtf");
    assert!(a.file_dialog_open(), "waiting on the system dialog");
    assert!(!request(&files, 0).fallback);
    a.run("ui.fileBrowser", json!({"purpose": "export"})).unwrap();
    assert_eq!(files.requests().len(), 2, "the pending dialog is given up and asked again");
    let again = request(&files, 1);
    assert!(again.fallback);
    assert!(again.file_name.as_deref().is_some_and(|n| n.ends_with(".rtf")), "{:?}", again.file_name);
    assert!(again.filters.iter().any(|f| f.extensions.iter().any(|e| e == "rtf")));
}

/// Word XML chosen as the export type wins over a Word 2003 XML file already at that path,
/// whose format a save by path alone would keep.
#[test]
fn exporting_word_xml_over_a_word_2003_file_writes_word_xml() {
    let dir = scratch("export_xml");
    let path = dir.join("letter.xml").to_string_lossy().into_owned();
    let mut a = WordApp::new(Session::new(wordcraft_doc::Document::new()), Services::default());
    a.run("text.insert", json!({"text": "Dear reader"})).unwrap();
    a.finish_pick(AfterPick::Export { ext: crate::print_page::EXPORT_WORD_2003.into() }, Some(path.clone())).unwrap();
    assert!(std::fs::read_to_string(&path).unwrap().contains("w:wordDocument"));
    a.finish_pick(AfterPick::Export { ext: "xml".into() }, Some(path.clone())).unwrap();
    let xml = std::fs::read_to_string(&path).unwrap();
    assert!(xml.contains("pkg:package") && !xml.contains("w:wordDocument"), "Word XML, not Word 2003 XML");
    let _ = std::fs::remove_dir_all(&dir);
}

/// A system Save dialog lets the user type a name without `.xml`; Word XML export still
/// writes `<name>.xml` and won't silently replace an existing one.
#[test]
fn exporting_word_xml_to_a_name_without_xml_appends_the_extension() {
    let dir = scratch("export_xml_ext");
    let mut a = WordApp::new(Session::new(wordcraft_doc::Document::new()), Services::default());
    a.run("text.insert", json!({"text": "Dear reader"})).unwrap();
    let bare = dir.join("letter").to_string_lossy().into_owned();
    a.finish_pick(AfterPick::Export { ext: "xml".into() }, Some(bare)).unwrap();
    let xml = std::fs::read_to_string(dir.join("letter.xml")).unwrap();
    assert!(xml.contains("pkg:package"), "Word XML written at letter.xml");
    // A known document extension gives way to .xml; an unknown one stays.
    let docx = dir.join("notes.docx").to_string_lossy().into_owned();
    a.finish_pick(AfterPick::Export { ext: "xml".into() }, Some(docx)).unwrap();
    assert!(dir.join("notes.xml").is_file());
    let v2 = dir.join("notes.v2").to_string_lossy().into_owned();
    a.finish_pick(AfterPick::Export { ext: "xml".into() }, Some(v2)).unwrap();
    assert!(dir.join("notes.v2.xml").is_file());
    // The resulting name already exists: the system dialog never confirmed it, so refuse.
    let bare = dir.join("letter").to_string_lossy().into_owned();
    assert!(a.finish_pick(AfterPick::Export { ext: "xml".into() }, Some(bare)).is_err());
    let _ = std::fs::remove_dir_all(&dir);
}

/// A name that is only a known extension (`.docx`) is a stem for `Path::extension`, which the
/// engine's explicit-format guard reads too: exporting Word XML to it must name a real
/// `.xml` file and write it (a bare `.xml` would be refused, writing nothing).
#[test]
fn exporting_word_xml_to_a_dotfile_name_writes_the_file() {
    let dir = scratch("export_xml_dotfile");
    let mut a = WordApp::new(Session::new(wordcraft_doc::Document::new()), Services::default());
    a.run("text.insert", json!({"text": "Dear reader"})).unwrap();
    for format in ["xml", crate::print_page::EXPORT_WORD_2003] {
        let typed = dir.join(".docx").to_string_lossy().into_owned();
        a.finish_pick(AfterPick::Export { ext: format.into() }, Some(typed)).unwrap();
        let written = dir.join(".docx.xml");
        assert!(std::fs::read_to_string(&written).unwrap().contains(if format == "xml" { "pkg:package" } else { "w:wordDocument" }));
        let _ = std::fs::remove_file(&written);
    }
    let _ = std::fs::remove_dir_all(&dir);
}

/// #1445: "Save changes?" → Save on a new document while another file dialog is still open:
/// nothing can be asked, so nothing happens, and the status bar says why.
#[test]
fn save_changes_while_a_file_dialog_is_open_says_so() {
    let (mut a, files) = async_app();
    typed(&mut a);
    a.open_dialog();
    assert_eq!(files.requests().len(), 1);
    a.run("file.close", json!({})).unwrap();
    let r = a.run("ui.saveChanges", json!({"answer": "save"})).unwrap();
    assert_eq!(r["done"], false);
    assert!(!a.quit_requested);
    assert!(a.session.dirty && body_text(&a).contains(UNSAVED));
    assert_eq!(files.requests().len(), 1, "nothing more was asked");
    let msg = status_text(&a);
    assert!(msg.contains("file dialog is already open"), "the status bar says why: {msg:?}");
}

/// "Save changes?" → Save on a new document: the held-back Close runs only once the later
/// Save As has actually saved; a cancelled Save As cancels it.
#[test]
fn save_changes_waits_for_the_save_as_answer() {
    let dir = scratch("prompt");
    let path = dir.join("novel.docx");
    let (mut a, files) = async_app();
    typed(&mut a);
    a.run("file.close", json!({})).unwrap();
    a.run("ui.saveChanges", json!({"answer": "save"})).unwrap();
    answer(&files, None);
    a.poll_file_dialog();
    assert!(body_text(&a).contains(UNSAVED), "cancelled Save As keeps the document");
    assert!(a.session.dirty);

    a.run("file.close", json!({})).unwrap();
    let r = a.run("ui.saveChanges", json!({"answer": "save"})).unwrap();
    assert_eq!(r["pending"], "saveAs");
    a.poll_file_dialog();
    assert!(a.has_document(), "nothing closes before the save");
    answer(&files, Some(&path.to_string_lossy()));
    a.poll_file_dialog();
    assert!(!a.has_document(), "closed once saved");
    assert!(saved_text(&path).contains(UNSAVED));
    let _ = std::fs::remove_dir_all(&dir);
}

/// Desktop (#240 on #94): Use an Existing List… asks the file service for a recipient list and,
/// once the answer arrives, loads it as the mail-merge recipients, not as a document.
#[test]
fn a_recipient_list_picked_later_loads_recipients() {
    let dir = scratch("recipients");
    let csv = dir.join("people.csv");
    std::fs::write(&csv, "First Name,City\nAda,London\nAlan,Wilmslow\n").unwrap();
    let (mut a, files) = async_app();
    typed(&mut a);
    a.run("ui.openRecipientList", json!({})).unwrap();
    assert_eq!(request(&files, 0).purpose, "recipients");
    assert!(a.file_dialog_open());
    answer(&files, Some(&csv.to_string_lossy()));
    a.poll_file_dialog().unwrap().unwrap();
    assert_eq!(a.session.merge.headers, vec!["First Name", "City"]);
    assert_eq!(a.session.merge.rows.len(), 2);
    assert!(body_text(&a).contains(UNSAVED), "the document stays open");
    // Cancelling loads nothing.
    a.run("ui.openRecipientList", json!({})).unwrap();
    answer(&files, None);
    assert_eq!(a.poll_file_dialog().unwrap().unwrap(), json!({"cancelled": true}));
    assert_eq!(a.session.merge.rows.len(), 2);
    let _ = std::fs::remove_dir_all(&dir);
}

/// #2640 (desktop): Manage Sources › Import… hands the picked Word bibliography file
/// (`Sources.xml`, `b:Sources`) to `references.sources`, and the status says how many
/// sources joined the document. Cancelling imports nothing, and importing the same file
/// twice adds nothing the second time.
#[test]
fn an_imported_sources_file_adds_its_sources() {
    let dir = scratch("sources");
    let xml = dir.join("Sources.xml");
    std::fs::write(
        &xml,
        r#"<b:Sources xmlns:b="http://schemas.openxmlformats.org/officeDocument/2006/bibliography">
  <b:Source><b:Tag>Riv21</b:Tag><b:SourceType>Book</b:SourceType><b:Title>Shared Spaces</b:Title><b:Year>2021</b:Year></b:Source>
</b:Sources>"#,
    )
    .unwrap();
    let (mut a, _) = async_app();
    a.finish_pick(AfterPick::Sources, Some(xml.to_string_lossy().to_string())).unwrap();
    assert_eq!(a.session.doc.sources.len(), 1);
    assert_eq!(a.session.doc.sources[0].tag, "Riv21");
    assert!(status_text(&a).contains("Imported 1 source"), "{}", status_text(&a));
    assert_eq!(a.finish_pick(AfterPick::Sources, None).unwrap(), json!({"cancelled": true}));
    a.finish_pick(AfterPick::Sources, Some(xml.to_string_lossy().to_string())).unwrap();
    assert_eq!(a.session.doc.sources.len(), 1, "the tag is already here");
    assert_eq!(status_text(&a), "No new sources in that file");
    let _ = std::fs::remove_dir_all(&dir);
}

/// A workbook with several sheets (#334) shows Select Table; its answer loads that sheet.
#[test]
fn a_workbook_with_several_sheets_asks_which_one() {
    let dir = scratch("workbook");
    let book = dir.join("people.ods");
    let table = |name: &str, cells: &[&str]| {
        let cells: String = cells
            .iter()
            .map(|c| {
                format!(r#"<table:table-row><table:table-cell office:value-type="string"><text:p>{c}</text:p></table:table-cell></table:table-row>"#)
            })
            .collect();
        format!(r#"<table:table table:name="{name}">{cells}</table:table>"#)
    };
    let content = format!(
        r#"<office:document-content xmlns:office="urn:oasis:names:tc:opendocument:xmlns:office:1.0" xmlns:table="urn:oasis:names:tc:opendocument:xmlns:table:1.0" xmlns:text="urn:oasis:names:tc:opendocument:xmlns:text:1.0"><office:body><office:spreadsheet>{}{}</office:spreadsheet></office:body></office:document-content>"#,
        table("Notes", &["n"]),
        table("Guests", &["Name", "Ada", "Alan"])
    );
    let mut z = craft_zip::Writer::new();
    z.add("mimetype", b"application/vnd.oasis.opendocument.spreadsheet", craft_zip::Compression::Stored).unwrap();
    z.add("content.xml", content.as_bytes(), craft_zip::Compression::Deflated).unwrap();
    std::fs::write(&book, z.finish().unwrap()).unwrap();
    let (mut a, files) = async_app();
    a.run("ui.openRecipientList", json!({})).unwrap();
    answer(&files, Some(&book.to_string_lossy()));
    a.poll_file_dialog().unwrap().unwrap();
    let open = a.dialog_host.open_dialogs();
    let select = open.iter().find(|d| d.key.as_deref() == Some("selectTable")).expect("Select Table expected");
    let fields = select.fields.as_ref().expect("Select Table lists its values");
    assert_eq!(fields["sheets"], json!(["Notes", "Guests"]));
    assert_eq!(fields["headers"], json!(true));
    assert!(a.session.merge.headers.is_empty(), "nothing loads before a sheet is chosen");
    a.load_recipients(json!({"path": book.to_string_lossy(), "headers": true, "sheet": 2})).unwrap();
    assert_eq!(a.session.merge.headers, vec!["Name"]);
    assert_eq!(a.session.merge.rows.len(), 2);
    let _ = std::fs::remove_dir_all(&dir);
}

/// The bundled face `file` of family `original` renamed `family` (as long as `original`), so it
/// is a font the database doesn't have.
fn renamed_font(file: &str, original: &str, family: &str) -> Vec<u8> {
    assert_eq!(family.len(), original.len());
    let mut data = std::fs::read(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/fonts").join(file)).unwrap();
    let utf16 = |s: &str| s.encode_utf16().flat_map(u16::to_be_bytes).collect::<Vec<u8>>();
    for (from, to) in [(utf16(original), utf16(family)), (original.as_bytes().to_vec(), family.as_bytes().to_vec())] {
        let mut i = 0;
        while let Some(at) = data.get(i..).and_then(|d| d.windows(from.len()).position(|w| w == from)) {
            data[i + at..i + at + to.len()].copy_from_slice(&to);
            i += at + to.len();
        }
    }
    data
}

fn font_families(a: &mut WordApp) -> Vec<String> {
    let list = a.run("font.list", json!({"all": true})).unwrap();
    list["families"].as_array().unwrap().iter().filter_map(|f| f.as_str().map(str::to_string)).collect()
}

/// #1789: Add Font asks for several files, and every one picked is added; an unreadable file
/// is reported and the rest still go in.
#[test]
fn add_font_imports_every_selected_file() {
    const ALPHA: &str = "Mfpick Alpha 1";
    const BRAVO: &str = "Mfpick Bravo 2";
    let dir = scratch("fonts");
    let alpha = dir.join("MfpickAlpha1-Regular.ttf");
    let junk = dir.join("Junk.ttf");
    let bravo = dir.join("MfpickBravo2-Regular.ttf");
    std::fs::write(&alpha, renamed_font("JetBrainsMono-Regular.ttf", "JetBrains Mono", ALPHA)).unwrap();
    std::fs::write(&junk, b"this is not a font").unwrap();
    std::fs::write(&bravo, renamed_font("JetBrainsMono-Regular.ttf", "JetBrains Mono", BRAVO)).unwrap();

    let (mut a, files) = async_app();
    a.pick_font();
    assert_eq!(request(&files, 0).kind, PickKind::OpenMany, "Add Font is multi-select");
    let paths: Vec<String> = [&alpha, &junk, &bravo].iter().map(|p| p.to_string_lossy().into_owned()).collect();
    answer_all(&files, &paths.iter().map(String::as_str).collect::<Vec<_>>());
    let r = a.poll_file_dialog().unwrap().unwrap();
    assert_eq!(r["added"], 2);
    assert_eq!(r["failed"].as_array().map(Vec::len), Some(1));
    let families = font_families(&mut a);
    assert!(families.iter().any(|f| f == ALPHA) && families.iter().any(|f| f == BRAVO), "both valid fonts were added");
    assert_eq!(status_text(&a), "Added 2 fonts; 1 couldn't be read: Junk.ttf");
    for f in [ALPHA, BRAVO] {
        a.run("font.remove", json!({"family": f})).unwrap();
    }
    let _ = std::fs::remove_dir_all(&dir);
}

/// The browser's picker answers with bytes, not paths: Open reads them as the document and Add
/// Font hands each font's data on.
#[test]
fn picked_contents_open_and_add_without_a_path() {
    let (mut a, files) = async_app();
    a.open_dialog();
    files.answer(Picked::Contents(vec![("web.txt".into(), b"Web hello".to_vec())]));
    a.poll_file_dialog().unwrap().unwrap();
    assert!(body_text(&a).contains("Web hello"), "the document came from the bytes");
    a.pick_font();
    files.answer(Picked::Contents(vec![("Junk.ttf".into(), b"not a font".to_vec())]));
    assert!(a.poll_file_dialog().unwrap().is_err(), "the font's data was read, and refused");
}

/// Only Add Font is multi-select; a cancelled pick adds nothing.
#[test]
fn only_add_font_asks_for_several_files() {
    let (mut a, files) = async_app();
    a.open_dialog();
    assert_eq!(request(&files, 0).kind, PickKind::Open);
    answer(&files, None);
    a.poll_file_dialog();
    a.pick_picture();
    assert_eq!(request(&files, 1).kind, PickKind::Open);
    answer(&files, None);
    a.poll_file_dialog();
    a.pick_font();
    let r = request(&files, 2);
    assert_eq!((r.kind, r.purpose.as_str()), (PickKind::OpenMany, "font"));
    answer_all(&files, &[]);
    assert_eq!(a.poll_file_dialog().unwrap().unwrap(), json!({"cancelled": true}));
    assert!(!a.file_dialog_open());
}

/// A hostile selection is bounded: only the first MAX_FONTS_PER_PICK are tried.
#[test]
fn a_huge_font_selection_is_capped() {
    let (mut a, _) = async_app();
    let fonts: Vec<(String, Value)> =
        (0..MAX_FONTS_PER_PICK + 50).map(|i| format!("/nonexistent/f{i}.ttf")).map(|p| (p.clone(), json!({"path": p}))).collect();
    let r = a.add_fonts(fonts).unwrap();
    assert_eq!(r["added"], 0);
    assert_eq!(r["failed"].as_array().map(Vec::len), Some(MAX_FONTS_PER_PICK));
    assert_eq!(r["skipped"], 50);
    assert_eq!(status_text(&a), "Added 0 fonts; 2000 couldn't be read: f0.ttf, f1.ttf, f2.ttf and 1997 more; 50 skipped (limit 2000)");
}

/// Past three failures the status line names three and counts the rest.
#[test]
fn many_failed_fonts_end_with_a_count() {
    let (mut a, _) = async_app();
    let fonts: Vec<(String, Value)> = (0..5).map(|i| format!("/nonexistent/f{i}.ttf")).map(|p| (p.clone(), json!({"path": p}))).collect();
    a.add_fonts(fonts).unwrap();
    assert_eq!(status_text(&a), "Added 0 fonts; 5 couldn't be read: f0.ttf, f1.ttf, f2.ttf and 2 more");
}
