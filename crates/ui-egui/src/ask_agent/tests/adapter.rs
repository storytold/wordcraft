#![allow(clippy::unwrap_used)]
use super::*;

#[test]
fn opening_a_bubble_closes_any_previous_request_in_the_hub() {
    let (mut app, hub) = app();
    hub.listen(7, None, Duration::ZERO, || true).unwrap();
    command(&mut app, &json!({"instruction": "First"})).unwrap();
    let first = hub.listen(7, None, Duration::ZERO, || true).unwrap().unwrap().id;
    assert!(hub.snapshot(first).is_some());
    command(&mut app, &json!({})).unwrap();
    assert!(hub.snapshot(first).is_none(), "opening a new bubble closed the earlier request");
}

#[test]
fn failed_command_does_not_record_edited_from_for_undo_join() {
    let (mut app, hub) = app();
    hub.listen(7, None, Duration::ZERO, || true).unwrap();
    command(&mut app, &json!({"instruction": "Edit"})).unwrap();
    let id = hub.listen(7, None, Duration::ZERO, || true).unwrap().unwrap().id;
    let before = app.session.undo_depth();
    // A failing command does not set edited_from
    let _ = as_agent(&mut app, Some(7), "text.insert", &json!({"request": id}), |a| a.run("invalid.command", json!({})));
    assert_eq!(app.ask_agent.open.as_ref().unwrap().edited_from, None);
    // Successful command afterwards sets edited_from
    let p = json!({"text": "done", "request": id});
    as_agent(&mut app, Some(7), "text.insert", &p, |a| a.run("text.insert", p.clone())).unwrap();
    assert_eq!(app.ask_agent.open.as_ref().unwrap().edited_from, Some(before));
}
use std::time::Duration;

fn app() -> (WordApp, AgentHub) {
    let mut app = WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::new()), Default::default());
    let hub = AgentHub::default();
    app.ask_agent.hub = Some(hub.clone());
    app.run("text.insert", json!({"text": "Kilns fire clay."})).unwrap();
    app.run("select.all", json!({})).unwrap();
    (app, hub)
}

#[test]
fn the_agents_edits_are_one_undo_step_authored_by_it_and_hold_the_text() {
    let (mut app, hub) = app();
    hub.listen(7, Some("Claude"), Duration::ZERO, || true).unwrap();
    command(&mut app, &json!({"instruction": "Shorter"})).unwrap();
    let id = hub.listen(7, Some("Claude"), Duration::ZERO, || true).unwrap().unwrap().id;
    let before = app.session.undo_depth();
    for text in ["Kilns ", "fire."] {
        let p = json!({"text": text, "request": id});
        as_agent(&mut app, Some(7), "text.insert", &p, |a| a.run("text.insert", p.clone())).unwrap();
    }
    assert_eq!(app.session.undo_depth(), before + 1, "one undo step");
    assert!(held(&app, "text.insert").unwrap().ends_with("(Claude)"), "the person's edit is refused");
    let other = as_agent(&mut app, Some(8), "text.insert", &json!({"text": "x"}), |a| a.run("text.insert", json!({"text": "x"})));
    assert!(other.unwrap_err().contains(hub::HELD), "another connection without the request");
    hub.reply(7, id, Reply { text: "Shortened.", ..Reply::default() }, Some(&current(&app.session))).unwrap();
    comment_once(&mut app, &hub.snapshot(id).unwrap());
    assert_eq!(app.session.undo_depth(), before + 1, "the closing comment joins the step");
    let comment = app.session.doc.comments.values().next().unwrap();
    assert_eq!(comment.author, "Agent (Claude)");
    let persisted = app.session.run("review.comments", &json!({})).unwrap().to_string();
    assert!(persisted.contains("Shorter") && persisted.contains("Shortened."), "comment retains instruction and answer");
    assert!(held(&app, "text.insert").is_none(), "answered: the text is free");
}

#[test]
fn a_follow_up_carries_the_thread_and_stays_in_the_same_undo_step() {
    let (mut app, hub) = app();
    hub.listen(1, None, Duration::ZERO, || true).unwrap();
    command(&mut app, &json!({"instruction": "Formal"})).unwrap();
    let first = hub.listen(1, None, Duration::ZERO, || true).unwrap().unwrap().id;
    hub.reply(1, first, Reply { text: "Made formal.", ..Reply::default() }, Some(&current(&app.session))).unwrap();
    send(&mut app, "Shorter".into()).unwrap();
    let second = hub.listen(1, None, Duration::ZERO, || true).unwrap().unwrap();
    assert_eq!(second.history, vec![Turn { instruction: "Formal".into(), answer: "Made formal.".into() }]);
    assert!(hub.snapshot(first).is_none(), "the answered request left the queue");
}

fn claim(app: &mut WordApp, hub: &AgentHub) -> u64 {
    let _ = hub.listen(7, Some("Claude"), Duration::ZERO, || true).unwrap();
    command(app, &json!({"instruction":"Improve this text"})).unwrap();
    hub.listen(7, Some("Claude"), Duration::ZERO, || true).unwrap().unwrap().id
}

fn edit(app: &mut WordApp, id: u64, text: &str) -> Result<Value, String> {
    let params = json!({"text":text,"request":id});
    as_agent(app, Some(7), "text.insert", &params, |app| app.run("text.insert", params.clone()))
}

#[test]
fn window_switch_never_applies_the_request_to_the_other_document() {
    let (mut app, hub) = app();
    let id = claim(&mut app, &hub);
    let original = app.session.document_id();
    app.session.run("file.new", &json!({})).unwrap();
    assert_ne!(app.session.document_id(), original);
    let before = app.session.doc.plain_text(wordcraft_doc::StoryRef::Body);
    assert!(edit(&mut app, id, "Must not reach the second document").unwrap_err().contains(hub::STALE_REQUEST));
    assert_eq!(app.session.doc.plain_text(wordcraft_doc::StoryRef::Body), before);
    assert!(!can_undo(&app));
}

#[test]
fn explicit_inactive_document_address_cannot_bypass_request_ownership() {
    let (mut app, hub) = app();
    let id = claim(&mut app, &hub);
    let first = app.session.document_id();
    app.session.run("file.new", &json!({})).unwrap();
    let second = app.session.document_id();
    app.session.switch_document(first).unwrap();
    let (mut request, _) =
        crate::ControlRequest::new("engine.execute", json!({"doc":second,"command":"text.insert","params":{"text":"wrong document"},"request":id}));
    request.connection = Some(7);
    let crate::control::Outcome::Done(result) = crate::control::handle(&mut app, &egui::Context::default(), &request) else {
        panic!("expected a command response")
    };
    assert_eq!(result["errorCode"], hub::STALE_REQUEST);
    let text = app.session.with_document(second, |session| session.doc.plain_text(wordcraft_doc::StoryRef::Body)).unwrap();
    assert!(!text.contains("wrong document"));
}

#[test]
fn a_changed_document_drops_replies_and_same_depth_replacement_never_owns_undo() {
    let (mut app, hub) = app();
    let id = claim(&mut app, &hub);
    edit(&mut app, id, "Agent edit").unwrap();
    let depth = app.session.undo_depth();
    let token = app.session.undo_token();
    assert!(app.session.undo());
    app.session.run("text.insert", &json!({"text":"Person's replacement"})).unwrap();
    assert_eq!(app.session.undo_depth(), depth, "depth alone could falsely join this checkpoint");
    assert_ne!(app.session.undo_token(), token);
    assert!(!can_undo(&app));
    let response = reply(&mut app, Some(7), &json!({"id":id,"text":"obsolete"})).unwrap();
    assert_eq!(response["errorCode"], hub::STALE_REQUEST);
    assert!(hub.snapshot(id).unwrap().answer.is_none());
}

#[test]
fn failed_request_is_closed_before_retry_is_queued() {
    let (mut app, hub) = app();
    let id = claim(&mut app, &hub);
    let response = reply(&mut app, Some(7), &json!({"id":id,"error":"Cannot complete this request"})).unwrap();
    assert_eq!(response["ok"], true);
    send(&mut app, "Retry".into()).unwrap();
    assert!(hub.snapshot(id).is_none());
    let next = hub.listen(7, Some("Claude"), Duration::ZERO, || true).unwrap().unwrap();
    assert_ne!(next.id, id);
}

#[test]
fn ordinary_text_in_a_table_is_distinct_from_a_whole_cell_selection() {
    let (mut app, hub) = app();
    app.session.run("insert.table", &json!({"rows":2,"cols":2})).unwrap();
    app.session.run("table.selectColumn", &json!({})).unwrap();
    assert!(app.session.cell_range().is_some() || app.session.column.is_some());
    let _ = hub.listen(7, None, Duration::ZERO, || true).unwrap();
    let error = command(&mut app, &json!({"instruction":"Format this column"})).unwrap_err();
    assert!(error.contains(hub::OUTSIDE_SELECTION));
    assert!(hub.listen(7, None, Duration::ZERO, || true).unwrap().is_none());
}

#[test]
fn owner_typing_cannot_overtype_or_autocorrect_text_outside_its_scope() {
    let mut app = WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::from_text("teh suffix")), Default::default());
    let hub = AgentHub::default();
    app.ask_agent.hub = Some(hub.clone());
    app.session.sel = wordcraft_engine::Selection::caret(Pos::body(0, 3));
    app.session.prefs.overtype = true;
    app.session.prefs.typing_replaces_selection = false;
    let id = claim(&mut app, &hub);
    edit(&mut app, id, " ").unwrap();
    assert_eq!(app.session.doc.plain_text(wordcraft_doc::StoryRef::Body), "teh  suffix");
    assert!(app.session.prefs.overtype);
    assert!(!app.session.prefs.typing_replaces_selection);
}

#[test]
fn a_follow_up_retains_the_rewritten_text_as_its_target() {
    let (mut app, hub) = app();
    let id = claim(&mut app, &hub);
    edit(&mut app, id, "First rewrite").unwrap();
    reply(&mut app, Some(7), &json!({"id":id,"proposal":"Not executed","format":"text","comment":"Review this"})).unwrap();
    send(&mut app, "Refine again".into()).unwrap();
    let next = hub.listen(7, Some("Claude"), Duration::ZERO, || true).unwrap().unwrap();
    assert_eq!(next.selection["text"], "First rewrite");
    assert!(next.history[0].answer.contains("Proposal (not applied)"));
    assert_eq!(app.session.doc.plain_text(wordcraft_doc::StoryRef::Body), "First rewrite");
    let (anchor, focus) = range(&next.selection).unwrap();
    app.session.sel = wordcraft_engine::Selection { anchor, focus };
    edit(&mut app, next.id, "Second rewrite").unwrap();
    assert_eq!(app.session.doc.plain_text(wordcraft_doc::StoryRef::Body), "Second rewrite");
    let objects = app.session.doc.para_at(&Pos::body(0, 0)).unwrap().objects.clone();
    assert_eq!(objects.iter().filter(|object| matches!(object, wordcraft_doc::InlineObject::CommentStart { .. })).count(), 1);
    assert_eq!(
        objects.iter().filter(|object| matches!(object, wordcraft_doc::InlineObject::CommentEnd { .. })).count(),
        1,
        "the earlier note keeps both anchors after the follow-up edit"
    );
}

#[test]
fn synthetic_input_cannot_press_keep_or_undo_on_behalf_of_the_person() {
    let (mut app, hub) = app();
    let id = claim(&mut app, &hub);
    reply(&mut app, Some(7), &json!({"id":id,"text":"Done"})).unwrap();
    for method in ["ui.key", "ui.click", "ui.press", "ui.release", "ui.drag", "ui.text", "ui.move", "ui.wheel", "ui.resize", "ui.focus"] {
        let (mut request, _) = crate::ControlRequest::new(method, json!({"key":"Z","ctrl":true}));
        request.connection = Some(7);
        let crate::control::Outcome::Done(result) = crate::control::handle(&mut app, &egui::Context::default(), &request) else {
            panic!("expected response")
        };
        assert_eq!(result["errorCode"], hub::HELD, "{method}");
    }
}

#[test]
fn caret_formatting_cannot_expand_into_an_uncaptured_word() {
    let (mut app, hub) = app();
    app.session.sel = wordcraft_engine::Selection::caret(Pos::body(0, 3));
    let id = claim(&mut app, &hub);
    let revision = app.session.rev();
    let params = json!({"request":id});
    let result = as_agent(&mut app, Some(7), "format.bold", &params, |app| app.run("format.bold", json!({})));
    assert!(result.unwrap_err().contains(hub::OUTSIDE_SELECTION));
    assert_eq!(app.session.rev(), revision);
}

#[test]
fn a_ui_command_cannot_bypass_the_guard_and_close_the_persons_conversation() {
    let (mut app, hub) = app();
    let id = claim(&mut app, &hub);
    let result = as_agent(&mut app, Some(7), "ui.askAgent", &json!({"request":id}), |app| app.execute("ui.askAgent", json!({"close":true})));
    assert!(result.unwrap_err().contains(hub::OUTSIDE_SELECTION));
    assert_eq!(app.ask_agent.open.as_ref().unwrap().id, Some(id));
    assert_eq!(hub.snapshot(id).unwrap().status, Status::Working { conn: 7 });
}

#[test]
fn undo_restores_exact_original_text_and_removes_only_the_agents_note() {
    let (mut app, hub) = app();
    let before_text = app.session.doc.plain_text(wordcraft_doc::StoryRef::Body);
    let before_comments = app.session.doc.comments.len();
    let id = claim(&mut app, &hub);
    edit(&mut app, id, "Kilns ").unwrap();
    edit(&mut app, id, "fire.").unwrap();
    reply(&mut app, Some(7), &json!({"id":id,"text":"Shortened.","proposal":"Unapplied alternative","comment":"Keep this note"})).unwrap();
    comment_once(&mut app, &hub.snapshot(id).unwrap());
    assert!(can_undo(&app));
    assert_eq!(app.session.doc.plain_text(wordcraft_doc::StoryRef::Body), "Kilns fire.");
    let comments = app.session.run("review.comments", &json!({})).unwrap().to_string();
    assert!(comments.contains("Proposal (not applied)") && comments.contains("Keep this note"));
    assert!(app.session.undo());
    close(&mut app);
    assert_eq!(app.session.doc.plain_text(wordcraft_doc::StoryRef::Body), before_text);
    assert_eq!(app.session.doc.comments.len(), before_comments);
}

#[test]
fn a_frontend_error_cannot_leave_an_undo_join_for_the_persons_next_edit() {
    let (mut app, hub) = app();
    let id = claim(&mut app, &hub);
    edit(&mut app, id, "Agent rewrite").unwrap();
    let before = app.session.undo_depth();
    let params = json!({"request":id,"text":"ignored"});
    assert!(as_agent(&mut app, Some(7), "text.insert", &params, |_| Err("frontend refused".into())).is_err());
    close(&mut app);
    app.session.close_typing();
    app.run("text.insert", json!({"text":" Person edit"})).unwrap();
    assert_eq!(app.session.undo_depth(), before + 1);
    assert!(app.session.undo());
    assert_eq!(app.session.doc.plain_text(wordcraft_doc::StoryRef::Body), "Agent rewrite");
}

#[test]
fn clear_formatting_cannot_reset_a_whole_paragraph_outside_the_text_scope() {
    let (mut app, hub) = app();
    let id = claim(&mut app, &hub);
    let revision = app.session.rev();
    let result = as_agent(&mut app, Some(7), "format.clear", &json!({"request":id}), |app| app.run("format.clear", json!({})));
    assert!(result.unwrap_err().contains(hub::OUTSIDE_SELECTION));
    assert_eq!(app.session.rev(), revision);
}

#[test]
fn placeholder_typing_cannot_replace_content_outside_the_captured_caret() {
    let (mut app, hub) = app();
    app.session.sel = wordcraft_engine::Selection::caret(Pos::body(0, 0));
    app.session.run("developer.richText", &json!({})).unwrap();
    let control = app.session.doc.control_ranges(wordcraft_doc::StoryRef::Body).into_iter().next().unwrap();
    app.session.sel = wordcraft_engine::Selection::caret(control.content_start());
    let id = claim(&mut app, &hub);
    let before = app.session.doc.plain_text(wordcraft_doc::StoryRef::Body);
    assert!(edit(&mut app, id, "No wider replacement").unwrap_err().contains(hub::OUTSIDE_SELECTION));
    assert_eq!(app.session.doc.plain_text(wordcraft_doc::StoryRef::Body), before);
}

#[test]
fn text_selection_inside_one_table_cell_can_be_rewritten_without_touching_neighbors() {
    let (mut app, hub) = app();
    app.session.run("insert.table", &json!({"rows":2,"cols":2})).unwrap();
    let at = app.session.sel.focus.clone();
    app.session.run("text.insert", &json!({"text":"Cell text"})).unwrap();
    app.session.sel.anchor = at.clone();
    let id = claim(&mut app, &hub);
    edit(&mut app, id, "Rewritten cell").unwrap();
    assert_eq!(app.session.doc.para_at(&at).unwrap().text, "Rewritten cell");
    let table = app.session.doc.body.iter().find_map(|block| block.as_table()).unwrap();
    assert_eq!(table.rows.len(), 2);
    assert_eq!(table.rows[0].cells.len(), 2);
    assert!(table.rows[0].cells[1].blocks.iter().all(|block| block.as_para().is_some_and(|para| para.text.is_empty())));
}

#[test]
fn a_plain_text_snapshot_does_not_hide_a_selected_text_box_from_scope_checks() {
    let (mut app, hub) = app();
    app.session.sel = wordcraft_engine::Selection::caret(Pos::body(0, 0));
    app.session.run("insert.textBox", &json!({"text":"Inside the box"})).unwrap();
    app.session.also_selected.clear();
    app.session.sel = wordcraft_engine::Selection { anchor: Pos::body(0, 0), focus: Pos::body(0, wordcraft_doc::para::OBJ.len_utf8()) };
    let _ = hub.listen(7, None, Duration::ZERO, || true).unwrap();
    assert!(command(&mut app, &json!({"instruction":"Format this selection"})).unwrap_err().contains(hub::OUTSIDE_SELECTION));
}

#[test]
fn a_failed_agent_with_partial_edits_can_keep_the_edits_and_failure_note() {
    let (mut app, hub) = app();
    let before = app.session.undo_depth();
    let id = claim(&mut app, &hub);
    edit(&mut app, id, "Partial result").unwrap();
    reply(&mut app, Some(7), &json!({"id":id,"error":"The remaining step failed"})).unwrap();
    assert!(can_undo(&app));
    keep(&mut app).unwrap();
    assert!(app.ask_agent.open.is_none());
    assert_eq!(app.session.doc.plain_text(wordcraft_doc::StoryRef::Body), "Partial result");
    assert_eq!(app.session.undo_depth(), before + 1);
    let comments = app.session.run("review.comments", &json!({})).unwrap().to_string();
    assert!(comments.contains("The remaining step failed"));
    assert!(app.session.undo());
    assert_eq!(app.session.doc.plain_text(wordcraft_doc::StoryRef::Body), "Kilns fire clay.");
}

#[test]
fn a_failed_agent_with_partial_edits_can_undo_without_affecting_the_persons_text() {
    let (mut app, hub) = app();
    let id = claim(&mut app, &hub);
    edit(&mut app, id, "Partial result").unwrap();
    reply(&mut app, Some(7), &json!({"id":id,"error":"Failed after editing"})).unwrap();
    assert!(can_undo(&app));
    assert!(app.session.undo());
    close(&mut app);
    assert_eq!(app.session.doc.plain_text(wordcraft_doc::StoryRef::Body), "Kilns fire clay.");
    assert!(app.session.doc.comments.is_empty());
}

#[test]
fn a_refused_review_note_keeps_the_bubble_and_undo_available() {
    let (mut app, hub) = app();
    let id = claim(&mut app, &hub);
    edit(&mut app, id, "Agent result").unwrap();
    reply(&mut app, Some(7), &json!({"id":id,"text":"Done"})).unwrap();
    app.session.doc.settings.protection = Some("forms".into());
    assert!(keep(&mut app).unwrap_err().contains("NOTE_NOT_SAVED"));
    assert!(app.ask_agent.open.is_some());
    assert!(can_undo(&app));
    assert!(app.session.doc.comments.is_empty());
}

#[test]
fn retry_after_partial_failure_keeps_the_rebased_target_and_failure_history() {
    let (mut app, hub) = app();
    let id = claim(&mut app, &hub);
    edit(&mut app, id, "Partial rewrite").unwrap();
    reply(&mut app, Some(7), &json!({"id":id,"error":"Could not finish"})).unwrap();
    send(&mut app, "Finish the rewrite".into()).unwrap();
    let next = hub.listen(7, None, Duration::ZERO, || true).unwrap().unwrap();
    assert_eq!(next.selection["text"], "Partial rewrite");
    assert!(next.history[0].answer.contains("Could not finish"));
    assert!(hub.snapshot(id).is_none());
    assert!(can_undo(&app));
}

#[test]
fn keep_after_partial_failure_and_an_outside_edit_never_claims_to_save_a_stale_note() {
    let (mut app, hub) = app();
    let id = claim(&mut app, &hub);
    edit(&mut app, id, "Partial result").unwrap();
    reply(&mut app, Some(7), &json!({"id":id,"error":"Failed after editing"})).unwrap();
    app.session.close_typing();
    app.session.run("text.insert", &json!({"text":" Person's edit"})).unwrap();
    let before = app.session.doc.plain_text(wordcraft_doc::StoryRef::Body);
    assert!(!can_undo(&app));
    assert!(keep(&mut app).is_err());
    assert!(app.ask_agent.open.is_some());
    assert!(app.session.doc.comments.is_empty());
    assert_eq!(app.session.doc.plain_text(wordcraft_doc::StoryRef::Body), before);
}
