//! Real dialog/menu input has priority over closing the underlying agent conversation.

use super::*;
use egui_kittest::kittest::Queryable;

/// Run WordApp's real logic/layout passes, including the production popup and dialog handling.
fn window(app: WordApp) -> egui_kittest::Harness<'static, WordApp> {
    let mut harness = egui_kittest::Harness::builder().with_size(egui::vec2(1440.0, 900.0)).build_ui_state(
        |ui, app: &mut WordApp| {
            let ctx = ui.ctx().clone();
            app.logic(&ctx);
            app.ui(ui);
        },
        app,
    );
    harness.run_steps(6);
    harness
}

/// Escape cancels the actual Font dialog without discarding a live, owned request behind it.
#[test]
fn review_escape_font_modal_keeps_the_owned_conversation() {
    let (mut app, hub) = app();
    let id = claim(&mut app, &hub);
    app.dialog = crate::dialogs::Dialog::open("font", &mut app);
    assert!(matches!(app.dialog, Some(crate::dialogs::Dialog::Font { .. })));
    assert!(app.modal_open());
    let mut h = window(app);
    h.key_press(egui::Key::Escape);
    h.run_steps(2);
    assert_eq!(h.state().ask_agent.open.as_ref().and_then(|open| open.id), Some(id), "the Font dialog's Escape discarded the agent request");
    assert!(h.state().dialog.is_none(), "Escape must still cancel the Font dialog");
    assert!(matches!(hub.snapshot(id).unwrap().status, Status::Working { conn: 7 }));
    h.key_press(egui::Key::Escape);
    h.run_steps(2);
    assert!(h.state().ask_agent.open.is_none(), "a fresh Escape outside the dialog still closes the conversation");
}

/// The Actions menu's Escape must preserve an answered request and its recoverable native edit.
#[test]
fn review_escape_actions_menu_keeps_answered_edits() {
    let (mut app, hub) = app();
    let id = claim(&mut app, &hub);
    edit(&mut app, id, "Recoverable native edit").unwrap();
    reply(&mut app, Some(7), &json!({"id":id,"text":"Review this change"})).unwrap();
    let mut h = window(app);
    let token = h.state().session.undo_token();
    h.get_by_label("Actions").click();
    h.run_steps(2);
    assert!(egui::Popup::is_any_open(&h.ctx), "the actual Actions menu must be open");
    h.key_press(egui::Key::Escape);
    h.run_steps(2);
    assert_eq!(h.state().ask_agent.open.as_ref().and_then(|open| open.id), Some(id), "menu Escape discarded the person's review");
    assert!(!egui::Popup::is_any_open(&h.ctx));
    assert!(hub.snapshot(id).is_some());
    assert_eq!(h.state().session.undo_token(), token);
    assert!(h.state().session.doc.plain_text(wordcraft_doc::StoryRef::Body).contains("Recoverable native edit"));
    h.key_press(egui::Key::Escape);
    h.run_steps(2);
    assert!(h.state().ask_agent.open.is_none());
}

/// The picker exists before Send: closing its real popup must retain the draft conversation.
#[test]
fn review_escape_agent_picker_keeps_the_unsent_draft() {
    let (mut app, hub) = app();
    hub.listen(7, Some("First"), Duration::ZERO, || true).unwrap();
    hub.listen(8, Some("Second"), Duration::ZERO, || true).unwrap();
    command(&mut app, &json!({})).unwrap();
    app.ask_agent.draft = "Keep this draft".into();
    let mut h = window(app);
    h.get_by_role_and_label(egui::accesskit::Role::ComboBox, "Agent").click();
    h.run_steps(2);
    assert!(egui::Popup::is_any_open(&h.ctx), "the actual agent picker must be open");
    h.key_press(egui::Key::Escape);
    h.run_steps(2);
    assert!(h.state().ask_agent.open.is_some(), "picker Escape discarded the unsent conversation");
    assert_eq!(h.state().ask_agent.draft, "Keep this draft");
    assert!(!egui::Popup::is_any_open(&h.ctx));
    h.key_press(egui::Key::Escape);
    h.run_steps(2);
    assert!(h.state().ask_agent.open.is_none());
}
