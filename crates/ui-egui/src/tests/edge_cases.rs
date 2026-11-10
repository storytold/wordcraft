use super::*;

fn rename_harness() -> egui_kittest::Harness<'static, WordApp> {
    let mut a = app();
    a.run("file.rename", json!({"name": "Original draft"})).unwrap();
    a.run("file.rename", json!({})).unwrap();
    if let Some(dialogs::Dialog::RenameDocument { name, .. }) = &mut a.dialog {
        *name = "Should never reach another document".into();
    }
    let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(1440.0, 900.0)).build_ui_state(
        |ui, app: &mut WordApp| {
            let ctx = ui.ctx().clone();
            app.logic(&ctx);
            app.ui(ui);
        },
        a,
    );
    h.run_steps(6);
    h
}

#[test]
fn edge_case_rename_dialog_cannot_follow_a_document_replacement() {
    for (id, params) in [("file.new", json!({})), ("file.open", json!({"path": "replacement.txt", "data": "TmV3IGRvY3VtZW50"}))] {
        let mut h = rename_harness();
        let original = h.state().session.document_id();
        h.state_mut().execute(id, params).unwrap();
        assert_ne!(h.state().session.document_id(), original);
        let before = h.state().title_stem();
        h.run_steps(2);
        assert!(h.state().dialog.is_none(), "{id}: a rename dialog belongs to its original document");
        assert_eq!(h.state().title_stem(), before);
        assert!(h.state().session.file_name.is_none());
    }
}

#[test]
fn edge_case_rename_error_then_escape_keeps_the_original_name() {
    let mut h = rename_harness();
    if let Some(dialogs::Dialog::RenameDocument { name, .. }) = &mut h.state_mut().dialog {
        *name = "../invalid".into();
    }
    h.run_steps(2);
    h.get_by_role_and_label(egui::accesskit::Role::Button, "Rename").click();
    h.run_steps(2);
    assert!(matches!(&h.state().dialog, Some(dialogs::Dialog::RenameDocument { message, .. }) if !message.is_empty()));
    assert_eq!(h.state().title_stem(), "Original draft");
    h.event(egui::Event::Key { key: egui::Key::Escape, physical_key: None, pressed: true, repeat: false, modifiers: Default::default() });
    h.run_steps(2);
    assert!(h.state().dialog.is_none());
    assert_eq!(h.state().title_stem(), "Original draft");
    assert!(!h.state().session.dirty);
}

#[test]
fn edge_case_edits_to_the_same_document_do_not_cancel_rename() {
    let mut h = rename_harness();
    let original = h.state().session.document_id();
    h.state_mut().execute("text.insert", json!({"text": "Content entered by a command"})).unwrap();
    h.run_steps(2);
    assert_eq!(h.state().session.document_id(), original);
    assert!(h.state().dialog.is_some());
    h.get_by_role_and_label(egui::accesskit::Role::Button, "Rename").click();
    h.run_steps(2);
    assert!(h.state().dialog.is_none());
    assert_eq!(h.state().title_stem(), "Should never reach another document");
    assert_eq!(h.state().session.doc.plain_text(wordcraft_doc::StoryRef::Body), "Content entered by a command");
}
