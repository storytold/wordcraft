//! Saving to RTF or ODT warns first when the format can't keep everything (#1316).

use serde_json::json;
use wordcraft_engine::Session;

use crate::{AutoSaveBlock, Services, WordApp};

const TEXT: &str = "Chapter one of the draft";

fn app() -> WordApp {
    WordApp::new(Session::new(wordcraft_doc::Document::new()), Services::default())
}

/// An app with typed text; `comment` adds a comment on it (which RTF and ODT can't keep).
fn draft(comment: bool) -> WordApp {
    let mut a = app();
    a.run("text.insert", json!({"text": TEXT})).unwrap();
    if comment {
        a.run("select.all", json!({})).unwrap();
        a.run("review.newComment", json!({"text": "Tighten this"})).unwrap();
    }
    assert!(a.session.dirty);
    a
}

fn prompt(a: &WordApp) -> Option<&'static str> {
    a.dialog.as_ref().map(|d| d.name())
}

fn scratch(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("wordcraft-losses-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// Answer the system Save As dialog with `picked` and record the names it suggested.
fn pick(a: &mut WordApp, picked: Vec<std::path::PathBuf>) -> std::rc::Rc<std::cell::RefCell<Vec<String>>> {
    let asked = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
    let seen = asked.clone();
    let picked = std::cell::RefCell::new(picked.into_iter());
    a.services.pick_save = Some(Box::new(move |name| {
        seen.borrow_mut().push(name.to_string());
        picked.borrow_mut().next().map(|p| p.to_string_lossy().to_string())
    }));
    asked
}

#[test]
fn save_as_rtf_with_a_comment_warns_and_continue_saves() {
    for ext in ["rtf", "odt"] {
        let dir = scratch(&format!("continue-{ext}"));
        let file = dir.join(format!("draft.{ext}"));
        let mut a = draft(true);
        pick(&mut a, vec![file.clone()]);
        let r = a.run("file.saveAs", json!({})).unwrap();
        assert_eq!(prompt(&a), Some("formatLosses"), "{ext}: warned before writing ({r})");
        assert!(!file.exists(), "{ext}: nothing written yet");
        assert!(a.session.dirty);
        match &a.dialog {
            Some(crate::dialogs::Dialog::FormatLosses { name, losses, .. }) => {
                assert_eq!(name, &format!("draft.{ext}"));
                assert_eq!(losses, &["comments".to_string()]);
            }
            _ => panic!("{ext}: the warning is open"),
        }
        let r = a.run("ui.formatLosses", json!({"answer": "continue"})).unwrap();
        assert_eq!(r["losses"], json!(["comments"]), "{ext}");
        assert_eq!(prompt(&a), None);
        assert!(file.exists(), "{ext}: saved");
        assert!(!a.session.dirty, "{ext}: Continue counts as saved, as in Word");
        assert_eq!(a.session.path.as_deref(), Some(file.as_path()));
        let _ = std::fs::remove_dir_all(&dir);
    }
}

#[test]
fn cancel_saves_nothing() {
    let dir = scratch("cancel");
    let file = dir.join("draft.rtf");
    let mut a = draft(true);
    pick(&mut a, vec![file.clone()]);
    a.run("file.saveAs", json!({})).unwrap();
    assert_eq!(prompt(&a), Some("formatLosses"));
    a.run("ui.formatLosses", json!({"answer": "cancel"})).unwrap();
    assert_eq!(prompt(&a), None);
    assert!(!file.exists());
    assert!(a.session.dirty, "still unsaved");
    assert_eq!(a.session.path, None);
    assert!(a.run("ui.formatLosses", json!({"answer": "cancel"})).is_err(), "no warning is open any more");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn keep_the_word_format_saves_as_docx_instead() {
    let dir = scratch("docx");
    let rtf = dir.join("draft.rtf");
    let docx = dir.join("draft.docx");
    let mut a = draft(true);
    let asked = pick(&mut a, vec![rtf.clone(), docx.clone()]);
    a.run("file.saveAs", json!({})).unwrap();
    a.run("ui.formatLosses", json!({"answer": "docx"})).unwrap();
    assert_eq!(asked.borrow().last().map(String::as_str), Some("draft.docx"), "Save As suggests the Word name");
    assert_eq!(prompt(&a), None);
    assert!(!rtf.exists(), "no RTF was written");
    assert!(docx.exists());
    assert!(!a.session.dirty);
    assert_eq!(a.session.path.as_deref(), Some(docx.as_path()));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_document_rtf_can_hold_saves_without_a_warning() {
    for ext in ["rtf", "odt"] {
        let dir = scratch(&format!("plain-{ext}"));
        let file = dir.join(format!("plain.{ext}"));
        let mut a = draft(false);
        pick(&mut a, vec![file.clone()]);
        a.run("file.saveAs", json!({})).unwrap();
        assert_eq!(prompt(&a), None, "{ext}: nothing to warn about");
        assert!(file.exists());
        assert!(!a.session.dirty);
        let _ = std::fs::remove_dir_all(&dir);
    }
}

/// Save (Ctrl+S) on a document whose file is RTF warns each time it has something to lose.
#[test]
fn save_to_the_documents_own_rtf_file_warns() {
    let dir = scratch("own");
    let file = dir.join("draft.rtf");
    let mut a = draft(false);
    a.run("file.save", json!({"path": file.to_string_lossy()})).unwrap();
    assert!(!a.session.dirty, "a plain document saves without asking");
    a.run("select.all", json!({})).unwrap();
    a.run("review.newComment", json!({"text": "Hm"})).unwrap();
    let before = std::fs::read(&file).unwrap();
    a.run("file.save", json!({})).unwrap();
    assert_eq!(prompt(&a), Some("formatLosses"));
    assert_eq!(std::fs::read(&file).unwrap(), before, "not written yet");
    a.run("ui.formatLosses", json!({"answer": "continue"})).unwrap();
    assert!(!a.session.dirty);
    let _ = std::fs::remove_dir_all(&dir);
}

/// Scripts, agents and the control channel never get a dialog: the save happens and its result
/// lists the losses.
#[test]
fn programmatic_saves_never_ask() {
    let dir = scratch("script");
    let file = dir.join("draft.odt");
    let mut a = draft(true);
    let r = a.execute("file.save", json!({"path": file.to_string_lossy()})).unwrap();
    assert_eq!(prompt(&a), None);
    assert_eq!(r["losses"], json!(["comments"]));
    assert!(file.exists());
    assert!(!a.session.dirty);
    let _ = std::fs::remove_dir_all(&dir);
}

/// "Save changes?" → Save on an RTF document with a comment warns too; Continue saves and then
/// does what was asked (here: New).
#[test]
fn the_save_changes_prompt_warns_before_saving_to_rtf() {
    for (answer, saved_as) in [("continue", "draft.rtf"), ("docx", "draft.docx"), ("cancel", "")] {
        let dir = scratch(&format!("prompt-{answer}"));
        let file = dir.join("draft.rtf");
        let mut a = draft(false);
        a.run("file.save", json!({"path": file.to_string_lossy()})).unwrap();
        a.run("select.all", json!({})).unwrap();
        a.run("review.newComment", json!({"text": "Hm"})).unwrap();
        let before = std::fs::read(&file).unwrap();
        pick(&mut a, vec![dir.join("draft.docx")]);
        a.run("file.new", json!({})).unwrap();
        assert_eq!(prompt(&a), Some("saveChanges"));
        a.run("ui.saveChanges", json!({"answer": "save"})).unwrap();
        assert_eq!(prompt(&a), Some("formatLosses"), "{answer}");
        assert_eq!(std::fs::read(&file).unwrap(), before, "{answer}: not written yet");
        a.run("ui.formatLosses", json!({"answer": answer})).unwrap();
        assert_eq!(prompt(&a), None, "{answer}");
        if saved_as.is_empty() {
            assert!(a.session.dirty, "cancel: the document stays, unsaved");
            assert_eq!(a.session.path.as_deref(), Some(file.as_path()));
        } else {
            assert!(dir.join(saved_as).exists(), "{answer}");
            assert_eq!(a.session.path, None, "{answer}: then New replaced the saved document");
            assert!(!a.session.doc.plain_text(wordcraft_doc::StoryRef::Body).contains(TEXT));
        }
        let _ = std::fs::remove_dir_all(&dir);
    }
}

/// Continue on the warning whose save then fails brings the Save Changes prompt back with the
/// held command, instead of dropping both.
#[test]
fn a_failed_continue_restores_the_save_changes_prompt() {
    let dir = scratch("failed-continue");
    let mut a = draft(true);
    pick(&mut a, vec![dir.join("no-such-folder").join("draft.rtf")]);
    a.run("file.new", json!({})).unwrap();
    assert_eq!(prompt(&a), Some("saveChanges"));
    a.run("ui.saveChanges", json!({"answer": "save"})).unwrap();
    assert_eq!(prompt(&a), Some("formatLosses"));
    assert!(a.run("ui.formatLosses", json!({"answer": "continue"})).is_err());
    assert_eq!(prompt(&a), Some("saveChanges"), "the prompt is back, still holding File New");
    assert!(a.session.dirty);
    assert!(a.session.doc.plain_text(wordcraft_doc::StoryRef::Body).contains(TEXT));
    let _ = std::fs::remove_dir_all(&dir);
}

/// A browser download of an RTF or ODT file reports what it left out, like a save to a path.
#[test]
fn a_browser_download_lists_the_losses() {
    for name in ["draft.rtf", "draft.odt"] {
        let mut a = draft(true);
        a.services.download = Some(Box::new(|_, _| Ok(())));
        let r = a.execute("file.saveAs", json!({"path": name})).unwrap();
        assert!(r.get("downloaded").is_some(), "{name}: {r}");
        assert_eq!(r["losses"], json!(["comments"]), "{name}");
    }
    let mut a = draft(true);
    a.services.download = Some(Box::new(|_, _| Ok(())));
    let r = a.execute("file.saveAs", json!({"path": "draft.docx"})).unwrap();
    assert!(r.get("losses").is_none(), "{r}");
}

/// AutoSave writes only files that keep everything: not an RTF that would lose a comment.
#[test]
fn autosave_skips_an_rtf_file_that_would_lose_something() {
    let dir = scratch("autosave");
    let file = dir.join("draft.rtf");
    let mut a = draft(false);
    a.run("file.save", json!({"path": file.to_string_lossy()})).unwrap();
    assert_eq!(a.autosave_block(), None, "a plain RTF document can be autosaved");
    assert_eq!(a.autosave_block(), None, "asked again (cached)");
    a.run("select.all", json!({})).unwrap();
    a.run("review.newComment", json!({"text": "Hm"})).unwrap();
    assert_eq!(a.autosave_block(), Some(AutoSaveBlock::Losses { name: "draft.rtf".into() }));
    assert!(a.autosave_block().is_some_and(|b| b.save_as_helps()));
    let _ = std::fs::remove_dir_all(&dir);
}

/// The warning on screen: Continue, Save as Word Document and Cancel are buttons; clicking
/// Continue saves.
#[test]
fn the_warning_answers_from_its_buttons() {
    use egui_kittest::kittest::Queryable;
    for (button, written, docx) in [("Continue", true, false), ("Save as Word Document", false, true), ("Cancel", false, false)] {
        let dir = scratch(&format!("click-{}", button.len()));
        let file = dir.join("draft.rtf");
        let mut a = draft(true);
        pick(&mut a, vec![file.clone(), dir.join("draft.docx")]);
        a.run("file.saveAs", json!({})).unwrap();
        assert_eq!(prompt(&a), Some("formatLosses"));
        let mut fonts = false;
        let mut h = egui_kittest::Harness::builder().build_ui_state(
            move |ui, app: &mut WordApp| {
                let ctx = ui.ctx().clone();
                if fonts {
                    crate::dialogs::show(app, &ctx);
                } else {
                    crate::theme::install_fonts_for(&ctx, false);
                    fonts = true;
                }
            },
            a,
        );
        for _ in 0..3 {
            h.step();
        }
        assert!(h.query_by_label("Comments").is_some() || h.query_by_label_contains("Comments").is_some(), "the loss is listed");
        h.get_by_label(button).click();
        for _ in 0..3 {
            h.step();
        }
        let a = h.state();
        assert_eq!(prompt(a), None, "{button}");
        assert_eq!(file.exists(), written, "{button}");
        assert_eq!(dir.join("draft.docx").exists(), docx, "{button}");
        let _ = std::fs::remove_dir_all(&dir);
    }
}

/// File › Export to RTF says "Exported" only once something was written: not while the warning
/// waits, and not after Cancel.
#[test]
fn export_reports_nothing_until_the_warning_is_answered() {
    let dir = scratch("export");
    let file = dir.join("draft.rtf");
    let mut a = draft(true);
    pick(&mut a, vec![file.clone()]);
    a.status_msg = None;
    a.export_dialog("rtf");
    assert_eq!(prompt(&a), Some("formatLosses"));
    let said = |a: &WordApp| a.status_msg.as_ref().is_some_and(|(m, _)| m.contains("Exported"));
    assert!(!said(&a), "nothing exported yet");
    a.run("ui.formatLosses", json!({"answer": "cancel"})).unwrap();
    assert!(!said(&a), "cancelled: nothing exported");
    assert!(!file.exists());
    // Continue writes the file and says so.
    pick(&mut a, vec![file.clone()]);
    a.export_dialog("rtf");
    a.run("ui.formatLosses", json!({"answer": "continue"})).unwrap();
    assert!(file.exists());
    assert!(said(&a), "exported after Continue");
    let _ = std::fs::remove_dir_all(&dir);
}

/// The AutoSave switch on a new document asks where to save; when that is an RTF file that
/// would lose something and the user saves a Word document instead, AutoSave comes on.
#[test]
fn save_as_word_from_the_autosave_switch_turns_autosave_on() {
    let dir = scratch("autosave-docx");
    let mut a = draft(true);
    a.run("file.autosave", json!({"value": false})).unwrap();
    assert!(!a.autosave);
    pick(&mut a, vec![dir.join("draft.rtf"), dir.join("draft.docx")]);
    let _ = a.save_as(crate::file_dialogs::AfterSave::AutoSave);
    assert_eq!(prompt(&a), Some("formatLosses"));
    a.run("ui.formatLosses", json!({"answer": "docx"})).unwrap();
    assert!(dir.join("draft.docx").exists());
    assert!(a.autosaves(), "AutoSave is on for the Word document");
    let _ = std::fs::remove_dir_all(&dir);
}

/// The keyboard answers the warning: Escape cancels, Enter on the focused Continue saves.
#[test]
fn the_warning_answers_from_the_keyboard() {
    for (key, written) in [(egui::Key::Escape, false), (egui::Key::Enter, true)] {
        let dir = scratch(&format!("keys-{key:?}"));
        let file = dir.join("draft.rtf");
        let mut a = draft(true);
        pick(&mut a, vec![file.clone()]);
        a.run("file.saveAs", json!({})).unwrap();
        assert_eq!(prompt(&a), Some("formatLosses"));
        let mut fonts = false;
        let mut h = egui_kittest::Harness::builder().build_ui_state(
            move |ui, app: &mut WordApp| {
                let ctx = ui.ctx().clone();
                if fonts {
                    crate::dialogs::show(app, &ctx);
                } else {
                    crate::theme::install_fonts_for(&ctx, false);
                    fonts = true;
                }
            },
            a,
        );
        for _ in 0..3 {
            h.step();
        }
        h.key_press(key);
        for _ in 0..3 {
            h.step();
        }
        let a = h.state();
        assert_eq!(prompt(a), None, "{key:?}");
        assert_eq!(file.exists(), written, "{key:?}");
        assert_eq!(a.session.dirty, !written, "{key:?}");
        let _ = std::fs::remove_dir_all(&dir);
    }
}

/// The warning takes the keyboard from the page, as "Save changes?" does (#1597): the canvas,
/// focused while typing, keeps Tab and the arrows for itself, yet Continue has focus when the
/// warning opens over it and the arrows move between the warning's buttons.
#[test]
fn the_warning_takes_focus_from_the_page() {
    use egui_kittest::kittest::Queryable;
    let dir = scratch("over-page");
    let file = dir.join("draft.rtf");
    let mut a = draft(true);
    pick(&mut a, vec![file.clone()]);
    a.canvas.want_focus = true;
    let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(1440.0, 900.0)).build_ui_state(
        |ui, app: &mut WordApp| {
            let ctx = ui.ctx().clone();
            app.logic(&ctx);
            app.ui(ui);
        },
        a,
    );
    for _ in 0..6 {
        h.step();
    }
    assert!(h.state().canvas.focused, "the page has keyboard focus");
    h.state_mut().run("file.saveAs", json!({})).unwrap();
    for _ in 0..3 {
        h.step();
    }
    assert_eq!(prompt(h.state()), Some("formatLosses"));
    let focused = |h: &egui_kittest::Harness<WordApp>, label: &str| h.query_all_by_label(label).any(|n| n.is_focused());
    assert!(focused(&h, "Continue"), "Continue has focus when the warning opens");
    h.key_press(egui::Key::ArrowRight);
    h.step();
    assert!(focused(&h, "Save as Word Document"), "the arrow moved to the next button");
    let _ = std::fs::remove_dir_all(&dir);
}
